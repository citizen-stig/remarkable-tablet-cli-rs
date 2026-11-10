# Official sample page

Source: Stewart McSporran (Scratchydisk) and contributors, [remarkable-rm](https://github.com/Scratchydisk/remarkable-rm/tree/403c345411dc451b6496339a617f2950bb5549fa/examples).

Pinned revision: `403c345411dc451b6496339a617f2950bb5549fa`. Imported 2026-11-10. MIT; [original notice](../LICENSE).

The upstream README identifies this image as a tablet export via the USB API. Input and reference are unmodified. Device model, firmware, template asset, font settings and capture date are unknown. The image includes ruled background, coloured pens/highlights, typed text and numbered lists.

## Invalid full-image pairing

The numbered-paragraph parser error is fixed, and the renderer now handles
anchored handwriting, explicit RGBA, glyph highlights and typed text. However,
this pair cannot establish full-image conformance: the upstream artifacts differ
in content, and the export includes a paper template absent from the input.

- PNG text: **"Text small and highlighted"**. Binary text: **"Text small and
  hilight"**. The binary's spelling was independently confirmed using upstream
  **rmscene 0.8.0** (`TextDocument.from_scene_item(read_tree(...).root_text)`),
  not merely our own parser. The same upstream reader finds 141 raw line blocks,
  of which 134 have live values; our assembled page has those 134 strokes.
- The PNG includes bottom handwriting **"Text left side"** absent from the
  supplied scene, and the smiley/exclamation drawing near the top also differs.
- Upstream history for both paths has only the initial import
  `8ce25c53dc5d58d330f39e3aa5542e8bcdb8502d`; there is no older matching pair to
  recover from that history.

`official_export_matches_sample_page` remains explicitly ignored as an invalid
pairing, rather than weakened until it passes. CI runs it only as a non-blocking
diagnostic, with evidence uploaded. Supply a matching `.rm`, `.content` and
background template/export before enabling the full-image assertion. The source
binary itself is exercised by an active semantic conformance test.

The original image, binary, hashes, and comparison thresholds remain unchanged.

| Path | SHA-256 |
| --- | --- |
| `examples/sample-page.rm` | `e28e37100c9640862ce2efa72092e7ea7ad932d3d614e18ff88ea61c63003828` |
| `examples/sample-tablet-export.png` | `bb96fbec5e5005e5253b39da435c2488d711edf7978dad9bb2b1cb736ffae561` |
