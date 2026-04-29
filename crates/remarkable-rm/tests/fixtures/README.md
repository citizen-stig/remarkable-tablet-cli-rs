# `.rm` Test Fixtures

Real `.rm` v6 files captured from a reMarkable 2 tablet on 2026-04-29. Each is
a single-page notebook hand-drawn to exercise a specific area of the parser.

Wire format: all four are v6 (`reMarkable .lines file, version=6`).

Tablet firmware: _TODO — fill in from `remarkable-cli connect --format json` →
`firmware_version`._

## Files

### `smoke.rm` — minimum viable file

Source UUID: `73d5b223-8422-4fd4-ab72-5e85bd0fd771`

Ground truth (fill in what you actually drew, in order):

1. _TODO — pen, color, rough shape_
2. _TODO_
3. _TODO_

### `pens-small.rm` — pen / color enum coverage

Source UUID: `94b0b54d-10ff-4b30-b47e-e36409d9e103`

Ground truth — list each stroke with the pen and color it was drawn with:

1. _TODO — pen, color_
2. _TODO_
3. _TODO_
4. _TODO_
5. _TODO_
6. _TODO_
7. _TODO_
8. _TODO_

### `edits.rm` — tombstones + moves

Source UUID: `7d55a139-393a-4786-9495-3508b3f1c210`

Ground truth:

- Strokes drawn (in order, before edits): _TODO_
- Stroke(s) erased (which one): _TODO_
- Stroke(s) moved with select tool (which one, where): _TODO_

### `layers.rm` — multi-layer scene tree

Source UUID: `e4fdd35a-b2d0-456f-a5fd-1af26ff7017d`

Ground truth:

- Layer 1 name: _TODO_, visible: yes/no, strokes: _TODO_
- Layer 2 name: _TODO_, visible: yes/no, strokes: _TODO_
- Layer 3 name: _TODO_, visible: yes/no, strokes: _TODO_

## Re-downloading

If a fixture is lost or needs a fresh capture:

```
remarkable-cli download <uuid> --output <name>.rm
```

Replace the file in this directory and update the ground truth above.
