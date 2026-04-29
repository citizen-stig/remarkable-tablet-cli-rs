# `.rm` Test Fixtures

Real `.rm` v6 files captured from a reMarkable 2 tablet on 2026-04-29. Each is
a single-page notebook hand-drawn to exercise a specific area of the parser.

Wire format: all four are v6 (`reMarkable .lines file, version=6`), with a
`SceneInfoBlock` reporting the standard 1404 × 1872 page size.

Tablet firmware: 3.x (exact version not recorded; the `.rm` files emit
`current_version=2` for stroke blocks, which has been the format since v3.1).

## Files

Ground truth below was obtained by parsing each fixture; the integration tests
in `tests/parse_page.rs` lock these counts in. To re-derive after a fixture
swap, run:

```
cargo run -q -p remarkable-rm --example dump_page -- tests/fixtures/<file>.rm
```

### `smoke.rm` — minimum viable file

Source UUID: `73d5b223-8422-4fd4-ab72-5e85bd0fd771`

- Layers: 1 (`"Layer 1"`, visible)
- Strokes: 3
- Pens used: BallpointV2, FinelinerV2, HighlighterV2 (one each)
- Colors used: Black, Blue, Highlight (one each)

### `pens-small.rm` — pen / color enum coverage

Source UUID: `94b0b54d-10ff-4b30-b47e-e36409d9e103`

- Layers: 1 (`"Layer 1"`, visible)
- Strokes: 9 — exactly the nine v2 tools, one each:
  PaintbrushV2, MechanicalPencilV2, PencilV2, BallpointV2, MarkerV2,
  FinelinerV2, HighlighterV2, Calligraphy, Shader.
- Colors used: Black ×3, Blue, Gray, Highlight ×2, Red ×2.

### `edits.rm` — tombstones + moves

Source UUID: `7d55a139-393a-4786-9495-3508b3f1c210`

- Layers: 1 (`"Layer 1"`, visible)
- Strokes after edits: 8 (some strokes were drawn then erased; the parser
  filters tombstoned `LineItem`s out of `Layer.lines`).
- All remaining strokes are BallpointV2 / Blue.

### `layers.rm` — multi-layer scene tree

Source UUID: `e4fdd35a-b2d0-456f-a5fd-1af26ff7017d`

- Layers: 3 (`"Layer 1"`, `"Layer 2"`, `"Layer 3"`, all visible)
- Strokes per layer: 2 / 3 / 4
- All strokes are BallpointV2 / Blue.

## Re-downloading

If a fixture is lost or needs a fresh capture:

```
remarkable-cli download <uuid> --output <name>.rm
```

Replace the file in this directory and update the ground truth above (and the
corresponding assertions in `tests/parse_page.rs`).
