//! Independent document bundles; see conformance/metadata/remarquee.

use std::path::PathBuf;

use remarkable_metadata::metadata::{FileType, parse_content, parse_metadata};

fn case_dir(case: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/metadata/remarquee")
        .join(case)
}

fn content_bytes(case: &str) -> Vec<u8> {
    let path = case_dir(case).join(format!("fake-cpages-{case}.content"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn assert_pdf_pages(case: &str, expected: &[&str]) {
    let content =
        parse_content(&content_bytes(case)).unwrap_or_else(|error| panic!("parse {case}: {error}"));
    assert_eq!(content.file_type, FileType::Pdf, "case: {case}");
    assert_eq!(content.page_count, None, "case: {case}");
    assert_eq!(
        content.effective_page_count(),
        Some(u32::try_from(expected.len()).unwrap()),
        "case: {case}"
    );
    let ids: Vec<String> = expected.iter().map(ToString::to_string).collect();
    assert_eq!(
        content.ordered_page_ids(),
        Some(ids.clone()),
        "case: {case}"
    );
    assert_eq!(
        content.page_templates(),
        ids.into_iter().map(|id| (id, None)).collect::<Vec<_>>(),
        "case: {case}"
    );
    assert!(
        case_dir(case)
            .join(format!("fake-cpages-{case}.pdf"))
            .is_file()
    );
}

#[test]
fn deleted_pages_do_not_contribute_to_order_or_inferred_count() {
    // Upstream content has keep0, deleted del1, keep2; no pageCount override.
    assert_pdf_pages("pdf-deleted-pages", &["keep0", "keep2"]);
}

#[test]
fn unannotated_pdf_keeps_all_logical_pages() {
    assert_pdf_pages("pdf-no-annotations", &["p0", "p1", "p2", "p3"]);
    let strokes = case_dir("pdf-no-annotations").join("fake-cpages-pdf-no-annotations");
    assert!(
        !strokes.exists(),
        "expected no annotations, got {strokes:?}"
    );
}

#[test]
fn empty_annotation_file_does_not_remove_a_logical_page() {
    assert_pdf_pages("pdf-v6-empty-rm", &["p0", "p1"]);
    let dir = case_dir("pdf-v6-empty-rm").join("fake-cpages-pdf-v6-empty-rm");
    let bytes = std::fs::read(dir.join("p0.rm")).unwrap();
    assert_eq!(bytes, b"reMarkable .lines file, version=6 ");
    assert!(!dir.join("p1.rm").exists());
}

#[test]
fn incomplete_generated_notebook_is_rejected_explicitly() {
    // This upstream generator omitted fileType. Do not silently classify it as PDF.
    let error = parse_content(&content_bytes("notebook-v6-empty")).unwrap_err();
    assert!(
        error.to_string().contains("fileType"),
        "expected missing fileType diagnostic, got {error}"
    );
}

#[test]
fn generated_metadata_stubs_are_not_valid_device_metadata() {
    for case in [
        "notebook-v6-empty",
        "pdf-deleted-pages",
        "pdf-no-annotations",
        "pdf-v6-empty-rm",
    ] {
        let path = case_dir(case).join(format!("fake-cpages-{case}.metadata"));
        let bytes = std::fs::read(&path).unwrap();
        let result = parse_metadata(&bytes);
        assert!(
            result.is_err(),
            "{}: expected rejection, got {result:?}",
            path.display()
        );
    }
}
