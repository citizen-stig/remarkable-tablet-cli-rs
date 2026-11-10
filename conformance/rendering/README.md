# Rendering conformance

Reference provenance determines the subdirectory:

- [tablet-exports](tablet-exports/): official PNG, PDF, or SVG exports paired with
  their source inputs; the strongest independent visual reference here.
- [tablet-thumbnails](tablet-thumbnails/): low-resolution tablet previews paired
  with inputs; useful for layout and broad stroke coverage.
- [third-party](third-party/): another renderer's output paired with inputs;
  useful for differential testing, with its limitations recorded.

Within each subdirectory use `<source-project>/<case-id>/`. Record dimensions,
orientation, crop, templates, fonts, and conversion settings. Compare at a fixed
resolution with a documented tolerance; do not use raw PDF-byte equality as a
visual comparison. Preserve the original reference when rasterizing PDF or SVG.

See the [source catalog and attribution requirements](../README.md).
