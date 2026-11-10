# Pencil reference comparisons

Paired `.rm` files and tablet thumbnails from **hwangso595 and eink-sync
contributors**, pinned to `33a686ba8ab7dec3fc22d036460b183a0301b4ed`.
See the [original MIT notice](LICENSE) and per-case READMEs for hashes and paths.
Reference origin and initial score limits are documented in upstream
[`test_renderer_quality.py`](https://github.com/hwangso595/eink-sync/blob/33a686ba8ab7dec3fc22d036460b183a0301b4ed/extraction/test_renderer_quality.py).

## Comparison contract

Render directly at the reference's native 384 × 512 resolution on white. Compare
grayscale mean absolute error and 8 × 8 block-mean differences with equal weight,
using upstream's score limits: 20 for heavy pencil (`p2`), 10 for mixed pressure
(`p3`). The block score is **not SSIM**, despite the name used upstream. We use
an integer RGB mean for these monochrome images; upstream uses PyMuPDF grayscale
conversion/resizing, so scores from the two implementations are not identical.

Also require at least 90% ink recall and precision, allowing two pixels of
positional tolerance in either direction. Pixels darker than grayscale 240 are
ink. This prevents a blank page from passing because of large white margins:
the `p3` blank control actually scores 6.16 under the original limit of 10.
The ink thresholds are explicit conformance targets, not fitted to current output.

## Current status: required comparisons pass

The renderer now uses local pressure, speed, direction, and point width instead
of median-width opaque strokes. Both full comparisons run in the default test
suite with the original score limits and ink thresholds unchanged:

| Case | Previous score | Current score | Maximum allowed |
| --- | ---: | ---: | ---: |
| `quicksheets-p2` | 54.02 | 9.65 | 20 |
| `quicksheets-p3` | 19.28 | 2.78 | 10 |

Ink recall is approximately 0.972 and 0.979, respectively; precision is 1.00
for both. The default suite also tests the metric itself (identity, blank output,
missing strokes, and displacement). These tolerances establish the stated
comparison contract, not pixel-perfect pencil grain.

```sh
cargo test -p remarkable-rm --test visual_conformance --locked
```

On mismatch, expected/actual/difference PNGs are written to
`target/conformance/rendering/<case>/`. Difference pixels are absolute RGB
differences on black. References are immutable; do not regenerate them to pass.

The separate official sample remains an opt-in diagnostic because its binary
and PNG represent different page revisions. See that case's README for evidence;
it is not a valid full-image fidelity oracle.
