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

## Current status: known fidelity failures

The renderer uses median-width solid paths. It does not reproduce pencil texture
or varying pressure. Initial measurements from this branch:

| Case | Score | Maximum allowed | Status |
| --- | ---: | ---: | --- |
| `quicksheets-p2` | 54.02 | 20 | Fails |
| `quicksheets-p3` | 19.28 | 10 | Fails |

The full appearance comparisons are explicitly `#[ignore]` with a reason. They
are **not passing conformance tests**. Two default visual tests do compare actual
renders against the references for ink geometry only, using the same 90% recall
and precision targets: both currently pass (recall 1.00; precision 0.969 for p2,
0.947 for p3). This establishes coarse stroke placement, not tone or texture.
Default tests also validate the reference files and comparison metric (identity,
blank output, missing strokes, and displacement).
CI additionally runs the ignored comparisons in a non-blocking diagnostic step
and uploads visual evidence; the defaults do not hide their measurements.

Run the actual comparisons, including the separate official-export case:

```sh
cargo test -p remarkable-rm --test visual_conformance --locked -- --ignored --nocapture
```

This command currently exits nonzero. On mismatch, expected/actual/difference
PNGs are written to `target/conformance/rendering/<case>/`. Difference pixels are
absolute RGB differences on black. Fix rendering, then remove the corresponding
ignore; do not relax thresholds or regenerate references to pass the tests.
