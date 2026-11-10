#![no_main]

use libfuzzer_sys::fuzz_target;
use remarkable_metadata::parse_content;

fuzz_target!(|data: &[u8]| {
    let Ok(content) = parse_content(data) else {
        return;
    };

    let _ = content.effective_page_count();
    let templates = content.page_templates();
    if let Some(ids) = content.ordered_page_ids() {
        assert_eq!(ids.len(), templates.len());
    }

    let bytes = serde_json::to_vec(&content).expect("serialize parsed content");
    let reparsed = parse_content(&bytes).unwrap_or_else(|e| {
        panic!(
            "round-trip failed: {e}; serialized: {}",
            String::from_utf8_lossy(&bytes)
        )
    });
    assert_eq!(reparsed.page_templates(), templates);
    assert_eq!(
        reparsed.effective_page_count(),
        content.effective_page_count()
    );
});
