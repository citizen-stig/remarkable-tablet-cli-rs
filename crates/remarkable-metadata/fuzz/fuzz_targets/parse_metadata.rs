#![no_main]

use libfuzzer_sys::fuzz_target;
use remarkable_metadata::{DocumentEntry, parse_metadata};

fuzz_target!(|data: &[u8]| {
    let Ok(raw) = parse_metadata(data) else {
        return;
    };

    let bytes = serde_json::to_vec(&raw).expect("serialize parsed metadata");
    let reparsed = parse_metadata(&bytes).unwrap_or_else(|e| {
        panic!(
            "round-trip failed: {e}; serialized: {}",
            String::from_utf8_lossy(&bytes)
        )
    });
    assert_eq!(
        bytes,
        serde_json::to_vec(&reparsed).expect("serialize reparsed metadata"),
        "round-trip not stable"
    );

    // Documents without .content are a legit error; folders/templates must build.
    let _ = DocumentEntry::from_raw(uuid::Uuid::nil(), raw, None);
});
