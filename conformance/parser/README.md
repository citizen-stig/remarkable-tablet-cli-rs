# Parser conformance

Place cases under `<source-project>/<case-id>/`, with `.rm` inputs and expected
parsed semantics: text, layer visibility/order, surviving strokes, pen/colour
values, CRDT ordering, page geometry, and treatment of unknown fields.

Start with the rmscene and rmc corpora. Cover firmware variations, tombstones,
text formatting, highlights, extended pages, and malformed input. Distinguish
supported inputs from cases expected to return a specific diagnostic.

Expected values must have independent evidence, not merely a dump from the
parser under test. Shared rendering inputs may be referenced by relative path.
See the [source catalog and attribution requirements](../README.md).
