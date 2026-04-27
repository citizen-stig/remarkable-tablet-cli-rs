use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// On-disk item type, matching the tablet's `.metadata` JSON exactly:
/// `"DocumentType"`, `"CollectionType"`, `"TemplateType"`. Use this for
/// anything that round-trips through xochitl's filesystem.
///
/// Not for CLI output — the JSON output schema uses a lowercase, renamed
/// projection (`folder`/`document`/`template`); see
/// [`crate::commands::common::ItemKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemType {
    #[serde(rename = "DocumentType")]
    Document,
    #[serde(rename = "CollectionType")]
    Collection,
    #[serde(rename = "TemplateType")]
    Template,
}

/// The file format of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    Pdf,
    Epub,
    Notebook,
}

/// Typed parent reference. Root and Trash are logical containers without UUIDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Parent {
    Root,
    Trash,
    Folder(Uuid),
}

impl Serialize for Parent {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Parent::Root => serializer.serialize_str(""),
            Parent::Trash => serializer.serialize_str("trash"),
            Parent::Folder(uuid) => serializer.serialize_str(&uuid.to_string()),
        }
    }
}

impl<'de> Deserialize<'de> for Parent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "" => Ok(Parent::Root),
            "trash" => Ok(Parent::Trash),
            other => {
                let uuid = Uuid::parse_str(other).map_err(serde::de::Error::custom)?;
                Ok(Parent::Folder(uuid))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Timestamp serde helpers
// ---------------------------------------------------------------------------

/// Coerce a JSON value into epoch-ms. reMarkable firmware emits timestamps
/// inconsistently across versions: older builds use JSON numbers, newer
/// ones wrap them in strings, some files use floats, and a few use `null`
/// or even `true`/`false` as a "not set" placeholder. Returns `None` when
/// the value carries no usable timestamp (null/bool/empty string).
fn json_value_to_epoch_ms<E: serde::de::Error>(v: serde_json::Value) -> Result<Option<i64>, E> {
    use serde_json::Value;
    match v {
        Value::Null | Value::Bool(_) => Ok(None),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Some(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Some(f as i64))
            } else {
                Err(E::custom(format!("unrepresentable number {n}")))
            }
        }
        Value::String(s) => {
            if s.is_empty() {
                Ok(None)
            } else {
                s.parse::<i64>()
                    .map(Some)
                    .map_err(|e| E::custom(format!("expected i64 in string, got {s:?}: {e}")))
            }
        }
        other => Err(E::custom(format!(
            "unexpected JSON type for timestamp: {other:?}"
        ))),
    }
}

fn deserialize_epoch_ms<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<DateTime<Utc>, D::Error> {
    let v = serde_json::Value::deserialize(deserializer)?;
    let ms = json_value_to_epoch_ms::<D::Error>(v)?.unwrap_or(0);
    Utc.timestamp_millis_opt(ms)
        .single()
        .ok_or_else(|| serde::de::Error::custom(format!("invalid epoch ms: {ms}")))
}

fn serialize_epoch_ms<S: Serializer>(dt: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_i64(dt.timestamp_millis())
}

fn deserialize_option_epoch_ms<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error> {
    let v = serde_json::Value::deserialize(deserializer)?;
    match json_value_to_epoch_ms::<D::Error>(v)? {
        None => Ok(None),
        Some(ms) => Utc
            .timestamp_millis_opt(ms)
            .single()
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid epoch ms: {ms}"))),
    }
}

fn epoch_zero() -> DateTime<Utc> {
    Utc.timestamp_millis_opt(0).single().expect("epoch zero")
}

fn default_parent_root() -> Parent {
    Parent::Root
}

fn serialize_option_epoch_ms<S: Serializer>(
    dt: &Option<DateTime<Utc>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match dt {
        Some(dt) => serializer.serialize_some(&dt.timestamp_millis()),
        None => serializer.serialize_none(),
    }
}

// ---------------------------------------------------------------------------
// Raw serde structs (match on-disk JSON exactly)
// ---------------------------------------------------------------------------

/// Raw `.metadata` JSON as stored on the tablet.
///
/// reMarkable's metadata schema has drifted across firmware versions —
/// fields come and go, types switch (numbers vs strings), and a third
/// `TemplateType` item kind appeared. Most fields here have defaults so
/// older or unfamiliar files still parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMetadata {
    pub visible_name: String,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    #[serde(default = "default_parent_root")]
    pub parent: Parent,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(
        default = "epoch_zero",
        deserialize_with = "deserialize_epoch_ms",
        serialize_with = "serialize_epoch_ms"
    )]
    pub last_modified: DateTime<Utc>,
    #[serde(
        default,
        rename = "metadatamodified",
        deserialize_with = "deserialize_option_epoch_ms",
        serialize_with = "serialize_option_epoch_ms"
    )]
    pub metadata_modified: Option<DateTime<Utc>>,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_option_epoch_ms",
        serialize_with = "serialize_option_epoch_ms"
    )]
    pub last_opened: Option<DateTime<Utc>>,
}

/// Raw `.content` JSON as stored on the tablet.
///
/// `page_count` is read directly from `pageCount` when present (newer schemas);
/// `pages` is captured opaquely so older schemas without `pageCount` can fall
/// back to its array length.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawContent {
    pub file_type: FileType,
    #[serde(default)]
    pub page_count: Option<u32>,
    #[serde(default)]
    pub pages: Option<Vec<serde_json::Value>>,
}

impl RawContent {
    /// Best-effort page count: prefer the explicit `pageCount` field; fall
    /// back to the length of the `pages` array; otherwise `None`.
    pub fn effective_page_count(&self) -> Option<u32> {
        self.page_count
            .or_else(|| self.pages.as_ref().map(|p| p.len() as u32))
    }
}

// ---------------------------------------------------------------------------
// Merged entry (UUID + metadata + content)
// ---------------------------------------------------------------------------

/// Combined document/folder entry used by tree, path resolver, and commands.
#[derive(Debug, Clone, Serialize)]
pub struct DocumentEntry {
    pub uuid: Uuid,
    pub visible_name: String,
    pub item_type: ItemType,
    pub parent: Parent,
    pub deleted: bool,
    pub pinned: bool,
    #[serde(serialize_with = "serialize_epoch_ms")]
    pub last_modified: DateTime<Utc>,
    pub version: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(serialize_with = "serialize_option_epoch_ms")]
    pub last_opened: Option<DateTime<Utc>>,
    pub file_type: Option<FileType>,
    pub page_count: Option<u32>,
}

impl DocumentEntry {
    pub fn is_root_child(&self) -> bool {
        self.parent == Parent::Root
    }

    pub fn is_trashed(&self) -> bool {
        self.parent == Parent::Trash || self.deleted
    }

    pub fn is_folder(&self) -> bool {
        self.item_type == ItemType::Collection
    }

    pub fn is_document(&self) -> bool {
        self.item_type == ItemType::Document
    }

    pub fn is_template(&self) -> bool {
        self.item_type == ItemType::Template
    }

    /// UUID of the parent folder, or `None` for root-level and trashed items.
    pub fn parent_uuid(&self) -> Option<Uuid> {
        match self.parent {
            Parent::Folder(u) => Some(u),
            Parent::Root | Parent::Trash => None,
        }
    }

    /// Sort key for ordering by type: folders < notebooks < PDFs < ePubs < unknown < templates.
    pub fn type_sort_key(&self) -> u8 {
        match (self.item_type, self.file_type) {
            (ItemType::Collection, _) => 0,
            (ItemType::Document, Some(FileType::Notebook)) => 1,
            (ItemType::Document, Some(FileType::Pdf)) => 2,
            (ItemType::Document, Some(FileType::Epub)) => 3,
            (ItemType::Document, None) => 4,
            (ItemType::Template, _) => 5,
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

pub fn parse_metadata(data: &[u8]) -> anyhow::Result<RawMetadata> {
    Ok(serde_json::from_slice(data)?)
}

pub fn parse_content(data: &[u8]) -> anyhow::Result<RawContent> {
    Ok(serde_json::from_slice(data)?)
}

/// Extract a UUID from a filename like `"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.metadata"`.
/// Returns `None` if the filename doesn't end with `.metadata` or the stem isn't a valid UUID.
pub fn extract_uuid(filename: &str) -> Option<Uuid> {
    let stem = filename.strip_suffix(".metadata")?;
    Uuid::parse_str(stem).ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_type_serde() {
        assert_eq!(
            serde_json::to_string(&ItemType::Document).unwrap(),
            r#""DocumentType""#
        );
        assert_eq!(
            serde_json::to_string(&ItemType::Collection).unwrap(),
            r#""CollectionType""#
        );
        assert_eq!(
            serde_json::from_str::<ItemType>(r#""DocumentType""#).unwrap(),
            ItemType::Document
        );
        assert_eq!(
            serde_json::from_str::<ItemType>(r#""CollectionType""#).unwrap(),
            ItemType::Collection
        );
    }

    #[test]
    fn file_type_serde() {
        for (variant, json) in [
            (FileType::Pdf, r#""pdf""#),
            (FileType::Epub, r#""epub""#),
            (FileType::Notebook, r#""notebook""#),
        ] {
            assert_eq!(serde_json::to_string(&variant).unwrap(), json);
            assert_eq!(serde_json::from_str::<FileType>(json).unwrap(), variant);
        }
    }

    #[test]
    fn parent_serde_root() {
        let p = Parent::Root;
        assert_eq!(serde_json::to_string(&p).unwrap(), r#""""#);
        assert_eq!(
            serde_json::from_str::<Parent>(r#""""#).unwrap(),
            Parent::Root
        );
    }

    #[test]
    fn parent_serde_trash() {
        let p = Parent::Trash;
        assert_eq!(serde_json::to_string(&p).unwrap(), r#""trash""#);
        assert_eq!(
            serde_json::from_str::<Parent>(r#""trash""#).unwrap(),
            Parent::Trash
        );
    }

    #[test]
    fn parent_serde_folder() {
        let uuid = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();
        let p = Parent::Folder(uuid);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, r#""aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee""#);
        let back: Parent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Parent::Folder(uuid));
    }

    #[test]
    fn parent_serde_invalid_uuid() {
        assert!(serde_json::from_str::<Parent>(r#""not-a-uuid""#).is_err());
    }

    #[test]
    fn parse_full_metadata() {
        let json = r#"{
            "visibleName": "Meeting Notes",
            "type": "DocumentType",
            "parent": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            "deleted": false,
            "pinned": true,
            "lastModified": 1710518400000,
            "metadatamodified": 1710518400000,
            "version": 1,
            "tags": ["work", "meetings"],
            "lastOpened": 1710604800000
        }"#;
        let m = parse_metadata(json.as_bytes()).unwrap();
        assert_eq!(m.visible_name, "Meeting Notes");
        assert_eq!(m.item_type, ItemType::Document);
        assert_eq!(
            m.parent,
            Parent::Folder(Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap())
        );
        assert!(!m.deleted);
        assert!(m.pinned);
        assert_eq!(m.last_modified.timestamp_millis(), 1710518400000);
        assert_eq!(
            m.metadata_modified.unwrap().timestamp_millis(),
            1710518400000
        );
        assert_eq!(m.version, 1);
        assert_eq!(m.tags, vec!["work", "meetings"]);
        assert_eq!(m.last_opened.unwrap().timestamp_millis(), 1710604800000);
    }

    #[test]
    fn parse_metadata_with_string_encoded_timestamps() {
        // Newer reMarkable firmware writes timestamps as JSON strings.
        let json = r#"{
            "visibleName": "Stringy",
            "type": "DocumentType",
            "parent": "",
            "deleted": false,
            "pinned": false,
            "lastModified": "1742725761861",
            "metadatamodified": "1742725761862",
            "version": 1,
            "lastOpened": "1742725999999"
        }"#;
        let m = parse_metadata(json.as_bytes()).unwrap();
        assert_eq!(m.last_modified.timestamp_millis(), 1742725761861);
        assert_eq!(
            m.metadata_modified.unwrap().timestamp_millis(),
            1742725761862
        );
        assert_eq!(m.last_opened.unwrap().timestamp_millis(), 1742725999999);
    }

    #[test]
    fn parse_metadata_tolerates_missing_optional_fields() {
        // Newer firmware can omit `deleted`, `pinned`, `version`, and even
        // timestamp fields. Make sure we don't blow up.
        let json = r#"{
            "visibleName": "Quirky",
            "type": "DocumentType",
            "parent": "",
            "lastModified": null,
            "metadatamodified": null,
            "lastOpened": null
        }"#;
        let m = parse_metadata(json.as_bytes()).unwrap();
        assert_eq!(m.visible_name, "Quirky");
        assert!(!m.deleted);
        assert!(!m.pinned);
        assert_eq!(m.version, 0);
        assert_eq!(m.last_modified.timestamp_millis(), 0);
        assert!(m.metadata_modified.is_none());
        assert!(m.last_opened.is_none());
    }

    #[test]
    fn parse_metadata_template_type() {
        let json = r#"{
            "visibleName": "Quad Grid",
            "type": "TemplateType",
            "parent": "",
            "deleted": false,
            "pinned": false,
            "lastModified": 1710000000000,
            "metadatamodified": 1710000000000,
            "version": 1
        }"#;
        let m = parse_metadata(json.as_bytes()).unwrap();
        assert_eq!(m.item_type, ItemType::Template);
    }

    #[test]
    fn parse_metadata_boolean_timestamp_treated_as_unset() {
        // Some firmware writes `false`/`true` as a "not-set" placeholder for
        // optional timestamps. Treat the same as null.
        let json = r#"{
            "visibleName": "Booly",
            "type": "DocumentType",
            "parent": "",
            "deleted": false,
            "pinned": false,
            "lastModified": false,
            "metadatamodified": true,
            "version": 1
        }"#;
        let m = parse_metadata(json.as_bytes()).unwrap();
        assert_eq!(m.last_modified.timestamp_millis(), 0);
        assert!(m.metadata_modified.is_none());
    }

    #[test]
    fn parse_metadata_float_timestamp() {
        let json = r#"{
            "visibleName": "Floaty",
            "type": "DocumentType",
            "parent": "",
            "deleted": false,
            "pinned": false,
            "lastModified": 1.742725761861e12,
            "metadatamodified": 1742725761861,
            "version": 1
        }"#;
        let m = parse_metadata(json.as_bytes()).unwrap();
        assert_eq!(m.last_modified.timestamp_millis(), 1742725761861);
    }

    #[test]
    fn parse_minimal_metadata() {
        let json = r#"{
            "visibleName": "Quick Note",
            "type": "DocumentType",
            "parent": "",
            "deleted": false,
            "pinned": false,
            "lastModified": 1710000000000,
            "metadatamodified": 1710000000000,
            "version": 1
        }"#;
        let m = parse_metadata(json.as_bytes()).unwrap();
        assert_eq!(m.visible_name, "Quick Note");
        assert_eq!(m.parent, Parent::Root);
        assert!(m.tags.is_empty());
        assert!(m.last_opened.is_none());
    }

    #[test]
    fn parse_content_pdf() {
        let json = r#"{"fileType": "pdf"}"#;
        let c = parse_content(json.as_bytes()).unwrap();
        assert_eq!(c.file_type, FileType::Pdf);
        assert_eq!(c.effective_page_count(), None);
    }

    #[test]
    fn parse_content_notebook() {
        let json = r#"{"fileType": "notebook"}"#;
        let c = parse_content(json.as_bytes()).unwrap();
        assert_eq!(c.file_type, FileType::Notebook);
    }

    #[test]
    fn parse_content_page_count_explicit() {
        let json = r#"{"fileType": "notebook", "pageCount": 5}"#;
        let c = parse_content(json.as_bytes()).unwrap();
        assert_eq!(c.page_count, Some(5));
        assert_eq!(c.effective_page_count(), Some(5));
    }

    #[test]
    fn parse_content_page_count_falls_back_to_pages_len() {
        let json = r#"{"fileType": "notebook", "pages": ["a","b","c"]}"#;
        let c = parse_content(json.as_bytes()).unwrap();
        assert_eq!(c.page_count, None);
        assert_eq!(c.effective_page_count(), Some(3));
    }

    #[test]
    fn parse_content_explicit_page_count_wins_over_pages() {
        let json = r#"{"fileType": "notebook", "pageCount": 10, "pages": ["a","b","c"]}"#;
        let c = parse_content(json.as_bytes()).unwrap();
        assert_eq!(c.effective_page_count(), Some(10));
    }

    #[test]
    fn extract_uuid_valid() {
        let uuid = extract_uuid("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.metadata");
        assert_eq!(
            uuid,
            Some(Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap())
        );
    }

    #[test]
    fn extract_uuid_not_metadata() {
        assert!(extract_uuid("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.content").is_none());
        assert!(extract_uuid("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.pdf").is_none());
        assert!(extract_uuid("random-file.txt").is_none());
    }

    #[test]
    fn extract_uuid_invalid_uuid() {
        assert!(extract_uuid("not-a-uuid.metadata").is_none());
    }

    #[test]
    fn document_entry_helpers() {
        let entry = DocumentEntry {
            uuid: Uuid::new_v4(),
            visible_name: "Test".into(),
            item_type: ItemType::Document,
            parent: Parent::Root,
            deleted: false,
            pinned: false,
            last_modified: Utc::now(),
            version: 1,
            tags: vec![],
            last_opened: None,
            file_type: Some(FileType::Pdf),
            page_count: Some(3),
        };
        assert!(entry.is_root_child());
        assert!(!entry.is_trashed());
        assert!(entry.is_document());
        assert!(!entry.is_folder());

        let folder = DocumentEntry {
            item_type: ItemType::Collection,
            parent: Parent::Trash,
            deleted: true,
            file_type: None,
            ..entry.clone()
        };
        assert!(folder.is_folder());
        assert!(folder.is_trashed());
    }

    #[test]
    fn type_sort_key_ordering() {
        let base = DocumentEntry {
            uuid: Uuid::new_v4(),
            visible_name: String::new(),
            item_type: ItemType::Collection,
            parent: Parent::Root,
            deleted: false,
            pinned: false,
            last_modified: Utc::now(),
            version: 1,
            tags: vec![],
            last_opened: None,
            file_type: None,
            page_count: None,
        };
        assert_eq!(base.type_sort_key(), 0); // folder

        let notebook = DocumentEntry {
            item_type: ItemType::Document,
            file_type: Some(FileType::Notebook),
            ..base.clone()
        };
        let pdf = DocumentEntry {
            file_type: Some(FileType::Pdf),
            ..notebook.clone()
        };
        let epub = DocumentEntry {
            file_type: Some(FileType::Epub),
            ..notebook.clone()
        };
        let unknown = DocumentEntry {
            file_type: None,
            ..notebook.clone()
        };

        assert!(base.type_sort_key() < notebook.type_sort_key());
        assert!(notebook.type_sort_key() < pdf.type_sort_key());
        assert!(pdf.type_sort_key() < epub.type_sort_key());
        assert!(epub.type_sort_key() < unknown.type_sort_key());
    }

    #[test]
    fn metadata_round_trip() {
        let json = r#"{
            "visibleName": "Test Doc",
            "type": "DocumentType",
            "parent": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            "deleted": false,
            "pinned": false,
            "lastModified": 1710518400000,
            "metadatamodified": 1710518400000,
            "version": 1,
            "tags": ["a"],
            "lastOpened": 1710604800000
        }"#;
        let parsed = parse_metadata(json.as_bytes()).unwrap();
        let serialized = serde_json::to_string(&parsed).unwrap();
        let reparsed = parse_metadata(serialized.as_bytes()).unwrap();
        assert_eq!(parsed.visible_name, reparsed.visible_name);
        assert_eq!(parsed.parent, reparsed.parent);
        assert_eq!(
            parsed.last_modified.timestamp_millis(),
            reparsed.last_modified.timestamp_millis()
        );
    }
}
