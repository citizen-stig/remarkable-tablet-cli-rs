# rmscene parser corpus

Thirteen unmodified `.rm` fixtures from **Rick Lupton and rmscene contributors**,
imported from revision `d7d86ca3a8ca4965d911886a1660bc8acf654c1a` on 2026-11-10.
The upstream repository distributes these test files under its [MIT license](LICENSE).
Each case directory records the original path, immutable source link and SHA-256.

## Coverage and independent expectations

The integration test is
[`crates/remarkable-rm/tests/conformance.rs`](../../../crates/remarkable-rm/tests/conformance.rs).
Run it from the workspace root:

```sh
cargo test -p remarkable-rm --test conformance --locked
```

Every fixture has an individually named parser-acceptance test. Three additional
tests check independent upstream expectations rather than snapshotting this
project's parser output:

- `Normal_AB`: text `AB`, one empty visible layer named `Layer 1`, CRDT IDs,
  plain paragraph style, text position and width. Values come from
  [`test_scene_stream.py::test_normal_ab`](https://github.com/ricklupton/rmscene/blob/d7d86ca3a8ca4965d911886a1660bc8acf654c1a/tests/test_scene_stream.py).
- `Color_and_tool_v3.14.4`: page size 1620 × 2160, established by
  [`test_scene_tree.py::test_scene_info_paper_size`](https://github.com/ricklupton/rmscene/blob/d7d86ca3a8ca4965d911886a1660bc8acf654c1a/tests/test_scene_tree.py).
- The same colour fixture: highlight-coloured lines use Shader; green-v2, cyan,
  or magenta lines use BallpointV2, following
  [`test_color_tool.py`](https://github.com/ricklupton/rmscene/blob/d7d86ca3a8ca4965d911886a1660bc8acf654c1a/tests/test_color_tool.py).
  Our check applies to surviving assembled lines and requires both tool groups
  to be exercised. Upstream does not assert that every permitted colour exists.

| Fixture | Edge case | Firmware recorded by upstream tests |
| --- | --- | --- |
| `Normal_AB` | Minimal typed text | 3.0 |
| `Normal_A_stroke_2_layers` | Typed text with strokes and layers | 3.0 |
| `Normal_A_stroke_2_layers_v3.2.2` | Updated text/stroke format | 3.2.2 |
| `Normal_A_stroke_2_layers_v3.3.2` | Inline formatting | 3.3.2 |
| `Bold_Heading_Bullet_Normal` | Multiple paragraph styles | 3.0 |
| `Lines_v2` | Version-2 stroke encoding | 3.1 |
| `Lines_v2_updated` | Additional line fields | 3.2 |
| `Wikipedia_highlighted_p1` | PDF glyph highlights | 3.1 |
| `Wikipedia_highlighted_p2` | PDF glyph highlights | 3.1 |
| `With_SceneInfo_Block` | Scene information | Upstream labels 3.4 as uncertain |
| `Color_and_tool_v3.14.4` | Larger canvas and new colours/tools | 3.14 (filename: 3.14.4) |
| `More_color_highlight_shader_v3.15.4.2` | Highlight/shader fields | 3.15 (filename: 3.15.4.2) |
| `test-crdt-ordering` | Concurrent text insertions | Unknown |

Firmware information comes from the pinned `test_scene_stream.py` version table.
Device model, orientation, template and original capture dates are not established
by this import. A filename or canvas size alone is not treated as proof of model.

## Limits of this first batch

Acceptance means `parse_page` succeeds, not that every block is represented in
the public `Page`. In particular, glyph highlights are currently skipped, and the
concurrent-insert fixture does not yet assert reconstructed text ordering. These
tests do not establish rendering fidelity, complete text semantics, or document
metadata support. Other categories have their own tests and status in the
[conformance catalog](../../README.md).

No Python package or network access is needed to run the Rust tests. Upstream
assertion logic informed these tests; its copyright notice is preserved in LICENSE.
