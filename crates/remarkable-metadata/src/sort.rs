/// Sort key applied when listing children of a folder.
///
/// `Name` (the default) keeps folders before documents and orders each group
/// case-insensitively; `Modified` is descending by `lastModified`; `Type`
/// groups by entry kind (folders → notebooks → PDFs → ePubs → templates),
/// then by name within each group.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "clap", value(rename_all = "lower"))]
pub enum SortField {
    #[default]
    Name,
    Modified,
    Type,
}
