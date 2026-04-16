use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// The type of item on the tablet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemType {
    #[serde(rename = "DocumentType")]
    Document,
    #[serde(rename = "CollectionType")]
    Collection,
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

fn deserialize_epoch_ms<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<DateTime<Utc>, D::Error> {
    let ms = i64::deserialize(deserializer)?;
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
    let ms: Option<i64> = Option::deserialize(deserializer)?;
    match ms {
        None => Ok(None),
        Some(ms) => Utc
            .timestamp_millis_opt(ms)
            .single()
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid epoch ms: {ms}"))),
    }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMetadata {
    pub visible_name: String,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub parent: Parent,
    pub deleted: bool,
    pub pinned: bool,
    #[serde(
        deserialize_with = "deserialize_epoch_ms",
        serialize_with = "serialize_epoch_ms"
    )]
    pub last_modified: DateTime<Utc>,
    #[serde(
        rename = "metadatamodified",
        deserialize_with = "deserialize_epoch_ms",
        serialize_with = "serialize_epoch_ms"
    )]
    pub metadata_modified: DateTime<Utc>,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawContent {
    pub file_type: FileType,
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

    /// Sort key for ordering by type: folders < notebooks < PDFs < ePubs < unknown.
    pub fn type_sort_key(&self) -> u8 {
        match (self.item_type, self.file_type) {
            (ItemType::Collection, _) => 0,
            (ItemType::Document, Some(FileType::Notebook)) => 1,
            (ItemType::Document, Some(FileType::Pdf)) => 2,
            (ItemType::Document, Some(FileType::Epub)) => 3,
            (ItemType::Document, None) => 4,
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
        assert_eq!(serde_json::from_str::<Parent>(r#""""#).unwrap(), Parent::Root);
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
        assert_eq!(m.metadata_modified.timestamp_millis(), 1710518400000);
        assert_eq!(m.version, 1);
        assert_eq!(m.tags, vec!["work", "meetings"]);
        assert_eq!(m.last_opened.unwrap().timestamp_millis(), 1710604800000);
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
    }

    #[test]
    fn parse_content_notebook() {
        let json = r#"{"fileType": "notebook"}"#;
        let c = parse_content(json.as_bytes()).unwrap();
        assert_eq!(c.file_type, FileType::Notebook);
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
