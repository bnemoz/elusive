//! Manual peak integration and per-window metrics.
//!
//! Everything here is a pure function over a [`Channel`] and a volume window, so
//! the analysis math can be checked against analytic answers (a triangle, a
//! Gaussian) with no file and no UI in the loop.
//!
//! **Units.** Areas are reported in *display* units × mL — e.g. mAU·mL when the
//! channel stores AU and carries `display_scale = 1000`. That is what a user reads
//! off the plot, so it is what a peak table should contain.

use crate::error::{Error, Result};
use crate::model::{BaselineMode, Channel, Fraction, PeakId, PeakResult, Sample};
use serde::{Deserialize, Serialize};

/// A point on the baseline-corrected curve, in display units.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    volume_ml: f64,
    /// Signal minus baseline.
    corrected: f64,
}

/// Evaluate a baseline at a given volume, in display units.
fn baseline_at(channel: &Channel, mode: BaselineMode, v0: f32, v1: f32, v: f64) -> Result<f64> {
    let scale = channel.display_scale as f64;
    match mode {
        BaselineMode::DropToZero => Ok(0.0),
        BaselineMode::LinearEndpoints => {
            let y0 = endpoint_value(channel, v0)? as f64 * scale;
            let y1 = endpoint_value(channel, v1)? as f64 * scale;
            Ok(interpolate_line(v0 as f64, y0, v1 as f64, y1, v))
        }
        BaselineMode::ValleyToValley { left_ml, right_ml } => {
            let y0 = endpoint_value(channel, left_ml)? as f64 * scale;
            let y1 = endpoint_value(channel, right_ml)? as f64 * scale;
            Ok(interpolate_line(left_ml as f64, y0, right_ml as f64, y1, v))
        }
    }
}

/// Value at a baseline anchor, clamped to the sampled range.
///
/// Unlike [`Channel::value_at_volume`] this clamps rather than returning `None`:
/// a user dragging a baseline handle a hair past the last sample should get the
/// last sample's value, not a failed integration.
fn endpoint_value(channel: &Channel, v: f32) -> Result<f32> {
    if let Some(y) = channel.value_at_volume(v) {
        return Ok(y);
    }
    let first = channel.samples.first().ok_or_else(|| Error::Integration {
        detail: format!("channel {} has no samples to integrate", channel.id),
    })?;
    let last = channel.samples[channel.samples.len() - 1];
    Ok(if v <= first.volume_ml {
        first.value
    } else {
        last.value
    })
}

fn interpolate_line(x0: f64, y0: f64, x1: f64, y1: f64, x: f64) -> f64 {
    let span = x1 - x0;
    if span.abs() < f64::EPSILON {
        return (y0 + y1) / 2.0;
    }
    y0 + (x - x0) / span * (y1 - y0)
}

/// Build the baseline-corrected curve across `[v0, v1]`.
///
/// The window edges are added as interpolated points so that the reported area
/// depends on the window the user dragged, not on where samples happen to fall.
fn corrected_curve(channel: &Channel, v0: f32, v1: f32, mode: BaselineMode) -> Result<Vec<Point>> {
    let scale = channel.display_scale as f64;
    let mut points: Vec<Point> = Vec::new();

    let mut push = |volume_ml: f64, raw: f64| -> Result<()> {
        let base = baseline_at(channel, mode, v0, v1, volume_ml)?;
        points.push(Point {
            volume_ml,
            corrected: raw * scale - base,
        });
        Ok(())
    };

    if let Some(y) = channel.value_at_volume(v0) {
        push(v0 as f64, y as f64)?;
    }
    for s in channel.samples_in_volume(v0, v1) {
        if !s.is_finite() || s.volume_ml <= v0 || s.volume_ml >= v1 {
            continue;
        }
        push(s.volume_ml as f64, s.value as f64)?;
    }
    if let Some(y) = channel.value_at_volume(v1) {
        push(v1 as f64, y as f64)?;
    }

    Ok(points)
}

/// Integrate `channel` over `[v_start, v_end]` under `baseline`.
///
/// Errors when the window is degenerate or falls entirely outside the channel's
/// sampled range — both are user mistakes worth a message rather than a silent
/// zero-area peak.
pub fn integrate_peak(
    id: PeakId,
    channel: &Channel,
    v_start: f32,
    v_end: f32,
    baseline: BaselineMode,
) -> Result<PeakResult> {
    let (v0, v1) = if v_start <= v_end {
        (v_start, v_end)
    } else {
        (v_end, v_start)
    };

    if !v0.is_finite() || !v1.is_finite() {
        return Err(Error::Integration {
            detail: "integration window contains a non-finite volume".into(),
        });
    }
    if (v1 - v0) <= 0.0 {
        return Err(Error::Integration {
            detail: format!("integration window [{v0}, {v1}] mL has zero width"),
        });
    }

    let points = corrected_curve(channel, v0, v1, baseline)?;
    if points.len() < 2 {
        return Err(Error::Integration {
            detail: format!(
                "no samples of channel {} fall inside [{v0}, {v1}] mL",
                channel.id
            ),
        });
    }

    let area = trapezoid_area(&points);

    let apex = points.iter().copied().fold(points[0], |best, p| {
        if p.corrected > best.corrected {
            p
        } else {
            best
        }
    });

    let fwhm = fwhm_of(&points, apex);

    Ok(PeakResult {
        id,
        channel_id: channel.id.clone(),
        v_start_ml: v0,
        v_end_ml: v1,
        baseline,
        area,
        height: apex.corrected,
        apex_volume_ml: apex.volume_ml as f32,
        fwhm_ml: fwhm,
        estimated_mw_kda: None,
    })
}

/// Trapezoidal integration over volume.
fn trapezoid_area(points: &[Point]) -> f64 {
    points
        .windows(2)
        .map(|w| {
            let dx = w[1].volume_ml - w[0].volume_ml;
            0.5 * (w[0].corrected + w[1].corrected) * dx
        })
        .sum()
}

/// Full width at half maximum, by linear interpolation on each flank.
///
/// Returns `None` when the curve does not descend to half height on both sides
/// inside the window — a truncated peak has no honest FWHM, and reporting the
/// window edge instead would understate the width without saying so.
fn fwhm_of(points: &[Point], apex: Point) -> Option<f32> {
    if apex.corrected <= 0.0 {
        return None;
    }
    let half = apex.corrected / 2.0;
    let apex_idx = points.iter().position(|p| p.volume_ml == apex.volume_ml)?;

    let mut left = None;
    for i in (1..=apex_idx).rev() {
        let (a, b) = (points[i - 1], points[i]);
        if a.corrected <= half && b.corrected >= half {
            left = Some(interpolate_line(
                a.corrected,
                a.volume_ml,
                b.corrected,
                b.volume_ml,
                half,
            ));
            break;
        }
    }

    let mut right = None;
    for i in apex_idx..points.len().saturating_sub(1) {
        let (a, b) = (points[i], points[i + 1]);
        if a.corrected >= half && b.corrected <= half {
            right = Some(interpolate_line(
                a.corrected,
                a.volume_ml,
                b.corrected,
                b.volume_ml,
                half,
            ));
            break;
        }
    }

    match (left, right) {
        (Some(l), Some(r)) if r > l => Some((r - l) as f32),
        _ => None,
    }
}

/// Per-well scalar shown on the 96-well plate heatmap (`design.md` §9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlateMetric {
    /// ∫ value dV over the fraction's volume window.
    IntegratedArea,
    MaxValue,
    MeanValue,
    /// Value at the midpoint of the window.
    ValueAtCenter,
}

impl PlateMetric {
    pub const ALL: [PlateMetric; 4] = [
        PlateMetric::IntegratedArea,
        PlateMetric::MaxValue,
        PlateMetric::MeanValue,
        PlateMetric::ValueAtCenter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PlateMetric::IntegratedArea => "Integrated area",
            PlateMetric::MaxValue => "Max value",
            PlateMetric::MeanValue => "Mean value",
            PlateMetric::ValueAtCenter => "Value at centre",
        }
    }

    /// Unit suffix given the channel's display unit.
    pub fn unit_suffix(self, display_unit: &str) -> String {
        match self {
            PlateMetric::IntegratedArea => format!("{display_unit}·mL"),
            _ => display_unit.to_string(),
        }
    }
}

/// Compute a metric for one channel over one volume window, in display units.
///
/// Returns `None` when the window holds no usable samples — that is genuinely
/// different from "the metric is zero", and the plate must render the two
/// differently (`IMPLEMENTATION_PLAN.md` Phase 4).
pub fn metric_over_window(channel: &Channel, v0: f32, v1: f32, metric: PlateMetric) -> Option<f64> {
    let (v0, v1) = if v0 <= v1 { (v0, v1) } else { (v1, v0) };
    if !v0.is_finite() || !v1.is_finite() {
        return None;
    }
    let scale = channel.display_scale as f64;

    match metric {
        PlateMetric::ValueAtCenter => {
            let mid = 0.5 * (v0 + v1);
            channel.value_at_volume(mid).map(|v| v as f64 * scale)
        }
        _ => {
            let mut pts: Vec<Point> = Vec::new();
            if let Some(y) = channel.value_at_volume(v0) {
                pts.push(Point {
                    volume_ml: v0 as f64,
                    corrected: y as f64 * scale,
                });
            }
            for s in channel.samples_in_volume(v0, v1) {
                if !s.is_finite() || s.volume_ml <= v0 || s.volume_ml >= v1 {
                    continue;
                }
                pts.push(Point {
                    volume_ml: s.volume_ml as f64,
                    corrected: s.value as f64 * scale,
                });
            }
            if let Some(y) = channel.value_at_volume(v1) {
                pts.push(Point {
                    volume_ml: v1 as f64,
                    corrected: y as f64 * scale,
                });
            }
            if pts.is_empty() {
                return None;
            }
            match metric {
                PlateMetric::IntegratedArea => Some(trapezoid_area(&pts)),
                PlateMetric::MaxValue => Some(
                    pts.iter()
                        .map(|p| p.corrected)
                        .fold(f64::NEG_INFINITY, f64::max),
                ),
                PlateMetric::MeanValue => {
                    // Volume-weighted so uneven sampling inside the window does not
                    // bias the mean towards densely-sampled stretches.
                    let span = pts[pts.len() - 1].volume_ml - pts[0].volume_ml;
                    if span > 0.0 {
                        Some(trapezoid_area(&pts) / span)
                    } else {
                        Some(pts.iter().map(|p| p.corrected).sum::<f64>() / pts.len() as f64)
                    }
                }
                PlateMetric::ValueAtCenter => unreachable!("handled above"),
            }
        }
    }
}

/// Metric for every fraction in a run, in fraction order.
pub fn metrics_for_fractions(
    channel: &Channel,
    fractions: &[Fraction],
    metric: PlateMetric,
) -> Vec<Option<f64>> {
    fractions
        .iter()
        .map(|f| {
            if !f.has_usable_window() {
                return None;
            }
            let (a, b) = f.volume_window();
            metric_over_window(channel, a, b, metric)
        })
        .collect()
}

/// Build a synthetic Gaussian channel. Public because both the unit tests here
/// and the app's "no run loaded" demo need the same generator.
pub fn synthetic_gaussian(
    channel: &mut Channel,
    center_ml: f32,
    sigma_ml: f32,
    amplitude: f32,
    v_max_ml: f32,
    n: usize,
) {
    channel.samples = (0..n)
        .map(|i| {
            let v = v_max_ml * i as f32 / (n - 1).max(1) as f32;
            let z = (v - center_ml) / sigma_ml;
            Sample::new(v * 60.0, v, amplitude * (-0.5 * z * z).exp())
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChannelId, ChannelKind};

    fn channel_from(points: &[(f32, f32)]) -> Channel {
        let mut c = Channel::new("test", "Test", ChannelKind::Uv);
        c.samples = points
            .iter()
            .map(|&(v, y)| Sample::new(v * 60.0, v, y))
            .collect();
        c
    }

    #[test]
    fn triangle_area_matches_the_analytic_value() {
        // Triangle of base 2 mL and height 10 → area 10.
        let c = channel_from(&[(0.0, 0.0), (1.0, 10.0), (2.0, 0.0)]);
        let p = integrate_peak(PeakId(1), &c, 0.0, 2.0, BaselineMode::DropToZero).unwrap();
        assert!((p.area - 10.0).abs() < 1e-6, "area = {}", p.area);
        assert!((p.height - 10.0).abs() < 1e-6);
        assert!((p.apex_volume_ml - 1.0).abs() < 1e-6);
    }

    #[test]
    fn triangle_fwhm_is_half_the_base() {
        let c = channel_from(&[(0.0, 0.0), (1.0, 10.0), (2.0, 0.0)]);
        let p = integrate_peak(PeakId(1), &c, 0.0, 2.0, BaselineMode::DropToZero).unwrap();
        // Half height (5) is reached at 0.5 mL and 1.5 mL.
        assert!((p.fwhm_ml.unwrap() - 1.0).abs() < 1e-5, "{:?}", p.fwhm_ml);
    }

    #[test]
    fn gaussian_area_matches_amplitude_times_sigma_root_two_pi() {
        let mut c = Channel::new("uv", "UV 280", ChannelKind::Uv);
        synthetic_gaussian(&mut c, 10.0, 0.5, 100.0, 20.0, 4001);
        let p = integrate_peak(PeakId(1), &c, 6.0, 14.0, BaselineMode::DropToZero).unwrap();

        let analytic = 100.0 * 0.5 * (2.0 * std::f64::consts::PI).sqrt();
        let rel = (p.area - analytic).abs() / analytic;
        assert!(rel < 1e-3, "area {} vs analytic {analytic}", p.area);
    }

    #[test]
    fn gaussian_fwhm_matches_two_root_two_ln_two_sigma() {
        let mut c = Channel::new("uv", "UV 280", ChannelKind::Uv);
        synthetic_gaussian(&mut c, 10.0, 0.5, 100.0, 20.0, 4001);
        let p = integrate_peak(PeakId(1), &c, 6.0, 14.0, BaselineMode::DropToZero).unwrap();

        let analytic = 2.0 * (2.0f32 * 2.0f32.ln()).sqrt() * 0.5;
        let got = p.fwhm_ml.unwrap();
        assert!(
            (got - analytic).abs() / analytic < 5e-3,
            "fwhm {got} vs {analytic}"
        );
    }

    #[test]
    fn linear_baseline_removes_a_sloping_offset() {
        // A triangle sitting on a ramp from 5 to 15: the ramp must integrate away.
        let pts: Vec<(f32, f32)> = (0..=200)
            .map(|i| {
                let v = i as f32 / 100.0; // 0..2 mL
                let ramp = 5.0 + 5.0 * v;
                let tri = if v <= 1.0 { 10.0 * v } else { 10.0 * (2.0 - v) };
                (v, ramp + tri)
            })
            .collect();
        let c = channel_from(&pts);
        let p = integrate_peak(PeakId(1), &c, 0.0, 2.0, BaselineMode::LinearEndpoints).unwrap();
        assert!((p.area - 10.0).abs() < 1e-3, "area = {}", p.area);
        assert!((p.height - 10.0).abs() < 1e-3, "height = {}", p.height);
    }

    #[test]
    fn drop_to_zero_and_linear_baselines_differ_on_an_offset_peak() {
        let pts: Vec<(f32, f32)> = (0..=200)
            .map(|i| {
                let v = i as f32 / 100.0;
                let tri = if v <= 1.0 { 10.0 * v } else { 10.0 * (2.0 - v) };
                (v, 5.0 + tri)
            })
            .collect();
        let c = channel_from(&pts);
        let zero = integrate_peak(PeakId(1), &c, 0.0, 2.0, BaselineMode::DropToZero).unwrap();
        let linear =
            integrate_peak(PeakId(2), &c, 0.0, 2.0, BaselineMode::LinearEndpoints).unwrap();
        // The constant 5 over 2 mL contributes exactly 10 extra area units.
        assert!((zero.area - linear.area - 10.0).abs() < 1e-3);
    }

    #[test]
    fn valley_to_valley_uses_the_supplied_anchors() {
        let c = channel_from(&[(0.0, 2.0), (1.0, 12.0), (2.0, 2.0)]);
        let vv = BaselineMode::ValleyToValley {
            left_ml: 0.0,
            right_ml: 2.0,
        };
        let p = integrate_peak(PeakId(1), &c, 0.0, 2.0, vv).unwrap();
        assert!((p.height - 10.0).abs() < 1e-6, "height = {}", p.height);
        assert!((p.area - 10.0).abs() < 1e-6, "area = {}", p.area);
    }

    #[test]
    fn display_scale_is_applied_to_area_and_height() {
        let mut c = channel_from(&[(0.0, 0.0), (1.0, 0.01), (2.0, 0.0)]);
        c.display_scale = 1000.0; // AU stored, mAU displayed
        let p = integrate_peak(PeakId(1), &c, 0.0, 2.0, BaselineMode::DropToZero).unwrap();
        assert!((p.height - 10.0).abs() < 1e-6, "height = {}", p.height);
        assert!((p.area - 10.0).abs() < 1e-6, "area = {}", p.area);
    }

    #[test]
    fn zero_width_window_is_an_error_not_a_zero_area_peak() {
        let c = channel_from(&[(0.0, 0.0), (1.0, 10.0), (2.0, 0.0)]);
        assert!(integrate_peak(PeakId(1), &c, 1.0, 1.0, BaselineMode::DropToZero).is_err());
    }

    #[test]
    fn window_outside_the_sampled_range_is_an_error() {
        let c = channel_from(&[(0.0, 0.0), (1.0, 10.0), (2.0, 0.0)]);
        assert!(integrate_peak(PeakId(1), &c, 50.0, 60.0, BaselineMode::DropToZero).is_err());
    }

    #[test]
    fn truncated_peak_reports_no_fwhm_rather_than_a_wrong_one() {
        // Window cut off before the signal falls back to half height on the right.
        let c = channel_from(&[(0.0, 0.0), (1.0, 10.0), (2.0, 9.0)]);
        let p = integrate_peak(PeakId(1), &c, 0.0, 2.0, BaselineMode::DropToZero).unwrap();
        assert_eq!(p.fwhm_ml, None);
    }

    #[test]
    fn reversed_window_is_normalised() {
        let c = channel_from(&[(0.0, 0.0), (1.0, 10.0), (2.0, 0.0)]);
        let a = integrate_peak(PeakId(1), &c, 0.0, 2.0, BaselineMode::DropToZero).unwrap();
        let b = integrate_peak(PeakId(1), &c, 2.0, 0.0, BaselineMode::DropToZero).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn plate_metrics_agree_with_hand_computed_values() {
        // Flat signal of 4.0 from 0 to 2 mL.
        let c = channel_from(&[(0.0, 4.0), (1.0, 4.0), (2.0, 4.0)]);
        assert_eq!(
            metric_over_window(&c, 0.0, 2.0, PlateMetric::IntegratedArea),
            Some(8.0)
        );
        assert_eq!(
            metric_over_window(&c, 0.0, 2.0, PlateMetric::MaxValue),
            Some(4.0)
        );
        assert_eq!(
            metric_over_window(&c, 0.0, 2.0, PlateMetric::MeanValue),
            Some(4.0)
        );
        assert_eq!(
            metric_over_window(&c, 0.0, 2.0, PlateMetric::ValueAtCenter),
            Some(4.0)
        );
    }

    #[test]
    fn mean_metric_is_volume_weighted_not_sample_weighted() {
        // A step: 101 densely-sampled points at 0 over the first half, then two
        // points at 10 covering the second half. Averaging over *samples* would
        // give ~0.2; averaging over *volume* gives 5, which is the honest answer
        // for a well whose signal was high for half its collection window.
        let mut pts: Vec<(f32, f32)> = (0..=100).map(|i| (i as f32 / 100.0, 0.0)).collect();
        pts.push((1.0001, 10.0));
        pts.push((2.0, 10.0));
        let c = channel_from(&pts);

        let mean = metric_over_window(&c, 0.0, 2.0, PlateMetric::MeanValue).unwrap();
        assert!((mean - 5.0).abs() < 0.01, "mean = {mean}");

        let naive = c.samples.iter().map(|s| s.value as f64).sum::<f64>() / c.samples.len() as f64;
        assert!(
            naive < 1.0,
            "the sample-weighted mean should differ: {naive}"
        );
    }

    #[test]
    fn metric_outside_the_sampled_range_is_none_not_zero() {
        let c = channel_from(&[(0.0, 4.0), (1.0, 4.0)]);
        assert_eq!(
            metric_over_window(&c, 50.0, 60.0, PlateMetric::IntegratedArea),
            None
        );
    }

    #[test]
    fn peak_carries_the_channel_it_was_integrated_on() {
        let c = channel_from(&[(0.0, 0.0), (1.0, 10.0), (2.0, 0.0)]);
        let p = integrate_peak(PeakId(7), &c, 0.0, 2.0, BaselineMode::DropToZero).unwrap();
        assert_eq!(p.channel_id, ChannelId::from("test"));
        assert_eq!(p.id, PeakId(7));
    }
}
