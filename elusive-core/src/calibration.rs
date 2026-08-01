//! SEC molecular-weight calibration and A280 concentration.
//!
//! These answer two different questions and are kept apart deliberately
//! (`design.md` §10): a calibration curve tells you *how big* something is, a
//! Beer–Lambert calculation tells you *how much* of it there is.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// One marker in a gel-filtration standard.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Standard {
    /// `A`..`E` as printed on the vial.
    pub letter: char,
    pub name: &'static str,
    pub mw_kda: f64,
    /// Amount per vial in mg, for reference when preparing the run.
    pub mg_per_vial: f64,
}

/// Bio-Rad Gel Filtration Standard, Cat# 1511901 (`design.md` §10).
///
/// Listed largest → smallest, which is also their elution order on a SEC column:
/// the biggest species is excluded from the pores and comes off first.
pub const BIORAD_GFS: [Standard; 5] = [
    Standard {
        letter: 'A',
        name: "Thyroglobulin (bovine)",
        mw_kda: 670.0,
        mg_per_vial: 5.0,
    },
    Standard {
        letter: 'B',
        name: "γ-globulin (bovine)",
        mw_kda: 158.0,
        mg_per_vial: 5.0,
    },
    Standard {
        letter: 'C',
        name: "Ovalbumin (chicken)",
        mw_kda: 44.0,
        mg_per_vial: 5.0,
    },
    Standard {
        letter: 'D',
        name: "Myoglobin (horse)",
        mw_kda: 17.0,
        mg_per_vial: 2.5,
    },
    Standard {
        letter: 'E',
        name: "Vitamin B12",
        mw_kda: 1.35,
        mg_per_vial: 0.5,
    },
];

/// What the calibration is fitted against.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum FitBasis {
    /// `log10(MW)` vs elution volume. Simple, but tied to this column geometry.
    ElutionVolume,
    /// `log10(MW)` vs `Kav = (Ve - V0) / (Vt - V0)`. Preferred when the column's
    /// void and total volumes are known, because it transfers between columns.
    Kav { v0_ml: f32, vt_ml: f32 },
}

impl FitBasis {
    pub fn label(self) -> &'static str {
        match self {
            FitBasis::ElutionVolume => "Elution volume",
            FitBasis::Kav { .. } => "Kav",
        }
    }

    /// Map an elution volume onto the fit's x-axis.
    pub fn x_for(self, ve_ml: f32) -> Option<f64> {
        match self {
            FitBasis::ElutionVolume => Some(ve_ml as f64),
            FitBasis::Kav { v0_ml, vt_ml } => {
                let denom = (vt_ml - v0_ml) as f64;
                (denom.abs() > f64::EPSILON).then(|| (ve_ml - v0_ml) as f64 / denom)
            }
        }
    }
}

/// A standard peak the user has assigned to a marker.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationPoint {
    pub mw_kda: f64,
    pub ve_ml: f32,
}

/// A fitted curve: `log10(MW) = slope * x + intercept`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub basis: FitBasis,
    pub slope: f64,
    pub intercept: f64,
    /// Coefficient of determination. Low values must be surfaced, not hidden
    /// (`IMPLEMENTATION_PLAN.md` Phase 7).
    pub r_squared: f64,
    pub points: Vec<CalibrationPoint>,
}

/// Fits below this R² are reported as low-confidence in the UI and in exports.
pub const LOW_CONFIDENCE_R2: f64 = 0.98;

impl Calibration {
    /// Estimate MW (kDa) for an elution volume.
    ///
    /// Returns `None` when the basis cannot map the volume (a degenerate
    /// `V0`/`Vt` pair) or the result is not a finite mass.
    pub fn mw_for_volume(&self, ve_ml: f32) -> Option<f64> {
        let x = self.basis.x_for(ve_ml)?;
        let log_mw = self.slope * x + self.intercept;
        let mw = 10f64.powf(log_mw);
        (mw.is_finite() && mw > 0.0).then_some(mw)
    }

    /// Whether the estimate for `ve_ml` requires extrapolating past the standards.
    /// Extrapolated sizes are not wrong so much as unsupported, and the UI must
    /// say which it is showing.
    pub fn is_extrapolated(&self, ve_ml: f32) -> bool {
        let Some(x) = self.basis.x_for(ve_ml) else {
            return true;
        };
        let xs: Vec<f64> = self
            .points
            .iter()
            .filter_map(|p| self.basis.x_for(p.ve_ml))
            .collect();
        match (
            xs.iter().copied().fold(f64::INFINITY, f64::min),
            xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        ) {
            (lo, hi) if lo <= hi => x < lo || x > hi,
            _ => true,
        }
    }

    pub fn is_low_confidence(&self) -> bool {
        self.r_squared < LOW_CONFIDENCE_R2
    }
}

/// Least-squares fit of `log10(MW)` against the chosen basis.
pub fn fit(points: &[CalibrationPoint], basis: FitBasis) -> Result<Calibration> {
    if points.len() < 2 {
        return Err(Error::Calibration {
            detail: format!(
                "a calibration needs at least 2 assigned standards, got {}",
                points.len()
            ),
        });
    }

    let mut xs = Vec::with_capacity(points.len());
    let mut ys = Vec::with_capacity(points.len());
    for p in points {
        if p.mw_kda <= 0.0 {
            return Err(Error::Calibration {
                detail: format!(
                    "standard has a non-positive molecular weight ({})",
                    p.mw_kda
                ),
            });
        }
        let x = basis.x_for(p.ve_ml).ok_or_else(|| Error::Calibration {
            detail: "Kav basis needs V0 and Vt to differ".to_string(),
        })?;
        if !x.is_finite() {
            return Err(Error::Calibration {
                detail: format!("standard at {} mL maps to a non-finite x", p.ve_ml),
            });
        }
        xs.push(x);
        ys.push(p.mw_kda.log10());
    }

    let n = xs.len() as f64;
    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = ys.iter().sum::<f64>() / n;

    let sxx: f64 = xs.iter().map(|x| (x - mean_x).powi(2)).sum();
    if sxx <= f64::EPSILON {
        return Err(Error::Calibration {
            detail: "all standards share the same elution volume; the fit is undefined".into(),
        });
    }
    let sxy: f64 = xs
        .iter()
        .zip(&ys)
        .map(|(x, y)| (x - mean_x) * (y - mean_y))
        .sum();

    let slope = sxy / sxx;
    let intercept = mean_y - slope * mean_x;

    let ss_tot: f64 = ys.iter().map(|y| (y - mean_y).powi(2)).sum();
    let ss_res: f64 = xs
        .iter()
        .zip(&ys)
        .map(|(x, y)| (y - (slope * x + intercept)).powi(2))
        .sum();
    // A perfectly flat set of MWs has no variance to explain; call that R² = 1
    // only when the residuals are also zero.
    let r_squared = if ss_tot <= f64::EPSILON {
        if ss_res <= f64::EPSILON {
            1.0
        } else {
            0.0
        }
    } else {
        1.0 - ss_res / ss_tot
    };

    Ok(Calibration {
        basis,
        slope,
        intercept,
        r_squared,
        points: points.to_vec(),
    })
}

/// Pre-fill an assignment from picked apex volumes.
///
/// SEC elutes largest first, so sorting the picked volumes ascending and pairing
/// them with the standards in descending mass order is the correct default. It is
/// a starting point for the user to correct, not an assertion.
pub fn suggest_assignment(apex_volumes_ml: &[f32]) -> Vec<CalibrationPoint> {
    let mut sorted: Vec<f32> = apex_volumes_ml
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    sorted.sort_by(f32::total_cmp);

    sorted
        .iter()
        .zip(BIORAD_GFS.iter())
        .map(|(&ve_ml, s)| CalibrationPoint {
            mw_kda: s.mw_kda,
            ve_ml,
        })
        .collect()
}

/// How the extinction coefficient was supplied.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Extinction {
    /// A(1%, 1 cm) style: absorbance of a 1 mg/mL solution through 1 cm.
    PerMgPerMl(f64),
    /// Molar extinction coefficient in M⁻¹cm⁻¹, needs the molecular weight.
    Molar { epsilon: f64, mw_da: f64 },
}

/// Beer–Lambert: `c = A / (ε · l)`.
///
/// Returns mg/mL for both extinction forms so the two paths are comparable.
pub fn concentration_mg_per_ml(
    absorbance_au: f64,
    extinction: Extinction,
    path_length_cm: f64,
) -> Result<f64> {
    if path_length_cm <= 0.0 {
        return Err(Error::Calibration {
            detail: format!("path length must be positive, got {path_length_cm} cm"),
        });
    }
    match extinction {
        Extinction::PerMgPerMl(e) => {
            if e <= 0.0 {
                return Err(Error::Calibration {
                    detail: "extinction coefficient must be positive".into(),
                });
            }
            Ok(absorbance_au / (e * path_length_cm))
        }
        Extinction::Molar { epsilon, mw_da } => {
            if epsilon <= 0.0 || mw_da <= 0.0 {
                return Err(Error::Calibration {
                    detail: "molar extinction and molecular weight must both be positive".into(),
                });
            }
            // mol/L → g/L is × MW; g/L and mg/mL are the same number.
            Ok(absorbance_au / (epsilon * path_length_cm) * mw_da)
        }
    }
}

/// Absorbance in AU from a peak apex height expressed in mAU.
pub fn au_from_mau(height_mau: f64) -> f64 {
    height_mau / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(v: &[(f64, f32)]) -> Vec<CalibrationPoint> {
        v.iter()
            .map(|&(mw_kda, ve_ml)| CalibrationPoint { mw_kda, ve_ml })
            .collect()
    }

    #[test]
    fn a_perfect_log_linear_series_fits_exactly() {
        // log10(MW) = -0.2 * Ve + 3.0, i.e. MW = 1000 kDa at Ve = 0.
        let points = pts(&[
            (10f64.powf(3.0 - 0.2 * 8.0), 8.0),
            (10f64.powf(3.0 - 0.2 * 12.0), 12.0),
            (10f64.powf(3.0 - 0.2 * 16.0), 16.0),
        ]);
        let cal = fit(&points, FitBasis::ElutionVolume).unwrap();
        assert!((cal.slope + 0.2).abs() < 1e-9, "slope {}", cal.slope);
        assert!((cal.intercept - 3.0).abs() < 1e-9);
        assert!((cal.r_squared - 1.0).abs() < 1e-12);
        assert!(!cal.is_low_confidence());
    }

    #[test]
    fn round_trips_a_molecular_weight_through_the_curve() {
        let points = pts(&[
            (670.0, 8.0),
            (158.0, 11.0),
            (44.0, 14.0),
            (17.0, 16.0),
            (1.35, 20.0),
        ]);
        let cal = fit(&points, FitBasis::ElutionVolume).unwrap();
        // The fit is real data-shaped, so check it lands in the right decade.
        let mw = cal.mw_for_volume(11.0).unwrap();
        assert!(mw > 50.0 && mw < 500.0, "mw = {mw}");
        assert!(cal.r_squared > 0.97, "r2 = {}", cal.r_squared);
    }

    #[test]
    fn kav_basis_normalises_by_column_geometry() {
        let basis = FitBasis::Kav {
            v0_ml: 8.0,
            vt_ml: 24.0,
        };
        // Ve = 16 mL sits exactly halfway between V0 and Vt.
        assert_eq!(basis.x_for(16.0), Some(0.5));
        assert_eq!(basis.x_for(8.0), Some(0.0));
        assert_eq!(basis.x_for(24.0), Some(1.0));
    }

    #[test]
    fn kav_with_equal_v0_and_vt_is_rejected_rather_than_dividing_by_zero() {
        let basis = FitBasis::Kav {
            v0_ml: 10.0,
            vt_ml: 10.0,
        };
        assert_eq!(basis.x_for(12.0), None);
        let err = fit(&pts(&[(670.0, 8.0), (158.0, 11.0)]), basis).unwrap_err();
        assert!(matches!(err, Error::Calibration { .. }));
    }

    #[test]
    fn fewer_than_two_standards_cannot_define_a_line() {
        assert!(fit(&pts(&[(670.0, 8.0)]), FitBasis::ElutionVolume).is_err());
        assert!(fit(&[], FitBasis::ElutionVolume).is_err());
    }

    #[test]
    fn identical_elution_volumes_are_rejected() {
        let err = fit(
            &pts(&[(670.0, 10.0), (158.0, 10.0)]),
            FitBasis::ElutionVolume,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Calibration { .. }));
    }

    #[test]
    fn a_scattered_fit_is_flagged_low_confidence() {
        let points = pts(&[(670.0, 8.0), (158.0, 9.0), (44.0, 8.5), (17.0, 20.0)]);
        let cal = fit(&points, FitBasis::ElutionVolume).unwrap();
        assert!(cal.is_low_confidence(), "r2 = {}", cal.r_squared);
    }

    #[test]
    fn extrapolation_beyond_the_standards_is_detectable() {
        let points = pts(&[(670.0, 8.0), (158.0, 12.0), (44.0, 16.0)]);
        let cal = fit(&points, FitBasis::ElutionVolume).unwrap();
        assert!(!cal.is_extrapolated(10.0));
        assert!(cal.is_extrapolated(4.0));
        assert!(cal.is_extrapolated(20.0));
    }

    #[test]
    fn suggested_assignment_pairs_earliest_peak_with_the_largest_standard() {
        let suggestion = suggest_assignment(&[16.0, 8.0, 12.0]);
        assert_eq!(suggestion.len(), 3);
        assert_eq!(suggestion[0].ve_ml, 8.0);
        assert_eq!(suggestion[0].mw_kda, 670.0);
        assert_eq!(suggestion[2].ve_ml, 16.0);
        assert_eq!(suggestion[2].mw_kda, 44.0);
    }

    #[test]
    fn beer_lambert_matches_a_hand_computed_case() {
        // A = 1.0 AU, ε = 2.0 (mg/mL)⁻¹cm⁻¹, l = 0.5 cm → 1.0 mg/mL.
        let c = concentration_mg_per_ml(1.0, Extinction::PerMgPerMl(2.0), 0.5).unwrap();
        assert!((c - 1.0).abs() < 1e-12, "c = {c}");
    }

    #[test]
    fn molar_extinction_converts_through_molecular_weight() {
        // A = 1.0, ε = 10000 M⁻¹cm⁻¹, l = 1 cm → 1e-4 M; at 50 kDa that is 5 mg/mL.
        let c = concentration_mg_per_ml(
            1.0,
            Extinction::Molar {
                epsilon: 10_000.0,
                mw_da: 50_000.0,
            },
            1.0,
        )
        .unwrap();
        assert!((c - 5.0).abs() < 1e-9, "c = {c}");
    }

    #[test]
    fn non_positive_inputs_are_errors_not_infinities() {
        assert!(concentration_mg_per_ml(1.0, Extinction::PerMgPerMl(2.0), 0.0).is_err());
        assert!(concentration_mg_per_ml(1.0, Extinction::PerMgPerMl(0.0), 1.0).is_err());
        assert!(concentration_mg_per_ml(
            1.0,
            Extinction::Molar {
                epsilon: 0.0,
                mw_da: 1.0
            },
            1.0
        )
        .is_err());
    }

    #[test]
    fn the_biorad_standard_is_recorded_largest_to_smallest() {
        let mws: Vec<f64> = BIORAD_GFS.iter().map(|s| s.mw_kda).collect();
        assert_eq!(mws, vec![670.0, 158.0, 44.0, 17.0, 1.35]);
        assert!(mws.windows(2).all(|w| w[0] > w[1]));
    }
}
