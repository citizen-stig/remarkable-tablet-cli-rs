//! 1-indexed page selections used by `download --pages`.
//!
//! Grammar: comma-separated list where each term is either a single page
//! `N` or an inclusive range `N-M` (with `N <= M`). All values must be
//! `>= 1`. Open-ended forms (`5-`, `-5`) are rejected because the parser
//! has no way to know the document length.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

/// A non-empty, deduplicated, sorted set of 1-indexed page numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSelection(BTreeSet<u32>);

impl PageSelection {
    #[must_use]
    pub fn contains(&self, page: u32) -> bool {
        self.0.contains(&page)
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PageSelectionError {
    #[error("page selection is empty")]
    Empty,
    #[error("page numbers are 1-indexed; `0` is not a valid page")]
    ZeroPage,
    #[error("invalid page number `{0}`")]
    InvalidNumber(String),
    #[error("range `{0}` has start greater than end")]
    InvertedRange(String),
    #[error("open-ended ranges like `{0}` are not supported; specify both endpoints")]
    OpenRange(String),
    #[error("expected a page number or `N-M` range, got `{0}`")]
    Malformed(String),
}

impl FromStr for PageSelection {
    type Err = PageSelectionError;

    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        let trimmed = spec.trim();
        if trimmed.is_empty() {
            return Err(PageSelectionError::Empty);
        }

        let mut pages = BTreeSet::new();
        for term in trimmed.split(',') {
            let term = term.trim();
            if term.is_empty() {
                return Err(PageSelectionError::Malformed(spec.to_string()));
            }

            if let Some((lhs, rhs)) = term.split_once('-') {
                let lhs = lhs.trim();
                let rhs = rhs.trim();
                if lhs.is_empty() || rhs.is_empty() {
                    return Err(PageSelectionError::OpenRange(term.to_string()));
                }
                let start = parse_page(lhs)?;
                let end = parse_page(rhs)?;
                if end < start {
                    return Err(PageSelectionError::InvertedRange(term.to_string()));
                }
                pages.extend(start..=end);
            } else {
                pages.insert(parse_page(term)?);
            }
        }

        if pages.is_empty() {
            return Err(PageSelectionError::Empty);
        }
        Ok(Self(pages))
    }
}

fn parse_page(text: &str) -> Result<u32, PageSelectionError> {
    let n: u32 = text
        .parse()
        .map_err(|_| PageSelectionError::InvalidNumber(text.to_string()))?;
    if n == 0 {
        return Err(PageSelectionError::ZeroPage);
    }
    Ok(n)
}

impl fmt::Display for PageSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for page in &self.0 {
            if !first {
                f.write_str(",")?;
            }
            write!(f, "{page}")?;
            first = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Vec<u32>, PageSelectionError> {
        PageSelection::from_str(s).map(|sel| sel.iter().collect())
    }

    #[test]
    fn single_page() {
        assert_eq!(parse("3").unwrap(), vec![3]);
    }

    #[test]
    fn comma_separated() {
        assert_eq!(parse("1,3,5").unwrap(), vec![1, 3, 5]);
    }

    #[test]
    fn closed_range() {
        assert_eq!(parse("2-5").unwrap(), vec![2, 3, 4, 5]);
    }

    #[test]
    fn mixed_terms() {
        assert_eq!(parse("1,3-5,7").unwrap(), vec![1, 3, 4, 5, 7]);
    }

    #[test]
    fn dedup_and_sort() {
        assert_eq!(parse("5,3,1,3,5").unwrap(), vec![1, 3, 5]);
    }

    #[test]
    fn whitespace_tolerated() {
        assert_eq!(parse(" 1 , 3 - 5 ").unwrap(), vec![1, 3, 4, 5]);
    }

    #[test]
    fn rejects_zero() {
        assert_eq!(parse("0").unwrap_err(), PageSelectionError::ZeroPage);
        assert_eq!(parse("0-3").unwrap_err(), PageSelectionError::ZeroPage);
        assert_eq!(parse("3-0").unwrap_err(), PageSelectionError::ZeroPage);
    }

    #[test]
    fn rejects_inverted_range() {
        assert!(matches!(
            parse("5-3"),
            Err(PageSelectionError::InvertedRange(_))
        ));
    }

    #[test]
    fn rejects_open_range() {
        assert!(matches!(parse("5-"), Err(PageSelectionError::OpenRange(_))));
        assert!(matches!(parse("-5"), Err(PageSelectionError::OpenRange(_))));
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(parse("").unwrap_err(), PageSelectionError::Empty);
        assert_eq!(parse("   ").unwrap_err(), PageSelectionError::Empty);
    }

    #[test]
    fn rejects_trailing_comma() {
        assert!(matches!(
            parse("1,2,"),
            Err(PageSelectionError::Malformed(_))
        ));
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(
            parse("abc"),
            Err(PageSelectionError::InvalidNumber(_))
        ));
    }

    #[test]
    fn contains_works() {
        let sel = PageSelection::from_str("2-4,8").unwrap();
        assert!(sel.contains(2));
        assert!(sel.contains(3));
        assert!(sel.contains(4));
        assert!(!sel.contains(5));
        assert!(sel.contains(8));
        assert!(!sel.contains(1));
    }

    #[test]
    fn display_round_trip() {
        let sel = PageSelection::from_str("3,1,2").unwrap();
        assert_eq!(sel.to_string(), "1,2,3");
    }
}
