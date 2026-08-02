# Test fixtures

`sec-run.ngcAnalysis` — a real Bio-Rad NGC SEC run (Superdex 200 10/300 GL,
16 channels, 75 fractions), passed through
`cargo run -p elusive-core --example sanitize_ngc` to redact identity fields.
Trace payloads are byte-identical to the instrument output; only free-text
metadata was replaced, so every number here is real.

`ColumnType` and `ColumnPosition` are deliberately left intact — they are
instrument configuration rather than sample identity, and the V0/Vt work in
`docs/format-findings.md` needs them.

Do not add a raw export here. `/test/` and `/testdata/private/` are gitignored
for exactly that reason.
