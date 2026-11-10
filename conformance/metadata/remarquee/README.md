# Document metadata cases from remarquee

Four synthetic document bundles from **Manuel Odendahl and go-go-golems/remarquee
contributors**, at revision `0694a8163ca1ca5f446ff610a961a80491451f84`.
The original [MIT notice](LICENSE) is preserved. Each case README records the
archive hash, extracted entry hashes and original paths. Entries are unmodified;
the original archive layout is retained inside each case directory.

Expected results come from inspecting the upstream JSON and archive directory,
not from this project's parser:

| Case | Independent expectation |
| --- | --- |
| `pdf-deleted-pages` | `del1` has `deleted.value = 1`; only `keep0`, `keep2` remain, in that order; inferred count 2. |
| `pdf-no-annotations` | Four logical pages `p0` through `p3`, despite no annotation directory. |
| `pdf-v6-empty-rm` | Two logical pages, although only `p0.rm` exists. The upstream "empty" file is a **truncated 33-byte header stub**, not a valid full v6 page. Metadata enumeration must not depend on parsing it. |
| `notebook-v6-empty` | `fileType` is missing; our strict content parser rejects it with a diagnostic naming the missing field. |

All generated `.metadata` files contain only `{"generated":true}`. Tests ensure
they are rejected as incomplete device metadata. These are intentionally
synthetic document cases, not evidence of device metadata compatibility.

Run:

```sh
cargo test -p remarkable-metadata --test conformance --locked
```

The source PDFs are document inputs, not reference exports. Tests cover logical
page enumeration, inferred counts, absent template values, and rejection of
incomplete content/metadata. The current model does not expose `redir`, so PDF
page redirection is **not yet asserted**. Real device metadata, template-valued
bundles, and end-to-end PDF composition remain further coverage to add.
