use glob::{MatchOptions, Pattern};

use crate::cli::{FindArgs, FindTypeFilter, GlobalOptions};
use crate::commands::common::{self, EntryView};
use crate::error::{CliError, Result};
use crate::metadata::DocumentEntry;
use crate::output::{self, OutputFormat};
use crate::tree::DocumentTree;

pub type FindItem = EntryView;

/// # Errors
/// Returns an error if connection fails, metadata cannot be loaded, or the pattern is an invalid glob.
pub async fn execute(global: &GlobalOptions, args: &FindArgs) -> Result<()> {
    run(global, args).await.map_err(common::to_cli_error)
}

async fn run(global: &GlobalOptions, args: &FindArgs) -> anyhow::Result<()> {
    let (ssh, cfg, tree) = common::connect_and_load_tree(global).await?;
    let result = run_with_tree(&tree, args);
    ssh.disconnect().await;
    let items = result?;
    print_output(&items, cfg.format);
    Ok(())
}

/// # Errors
/// Returns an error if `args.pattern` contains glob metacharacters but isn't a valid glob.
pub fn run_with_tree(tree: &DocumentTree, args: &FindArgs) -> anyhow::Result<Vec<FindItem>> {
    let matcher = build_matcher(&args.pattern, args.case_sensitive)?;
    let type_filter = args.item_type.as_ref();

    let mut items: Vec<FindItem> = tree
        .all_entries()
        .filter(|e| !e.is_trashed())
        .filter(|e| match type_filter {
            None | Some(FindTypeFilter::All) => true,
            Some(FindTypeFilter::Document) => e.is_document(),
            Some(FindTypeFilter::Folder) => e.is_folder(),
        })
        .filter(|e| matcher.matches(&e.visible_name))
        .map(|e| to_find_item(tree, e))
        .collect();

    items.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(items)
}

fn to_find_item(tree: &DocumentTree, e: &DocumentEntry) -> FindItem {
    EntryView::from_entry(tree, e)
}

// ---------------------------------------------------------------------------
// Matcher
// ---------------------------------------------------------------------------

enum Matcher {
    Substring {
        needle: String,
        case_sensitive: bool,
    },
    Glob {
        pattern: Pattern,
        opts: MatchOptions,
    },
}

impl Matcher {
    fn matches(&self, name: &str) -> bool {
        match self {
            Matcher::Substring {
                needle,
                case_sensitive,
            } => {
                if *case_sensitive {
                    name.contains(needle.as_str())
                } else {
                    name.to_lowercase().contains(needle.as_str())
                }
            }
            Matcher::Glob { pattern, opts } => pattern.matches_with(name, *opts),
        }
    }
}

fn build_matcher(pattern: &str, case_sensitive: bool) -> anyhow::Result<Matcher> {
    if pattern.contains('*') || pattern.contains('?') {
        let p = Pattern::new(pattern)
            .map_err(|e| CliError::InvalidPath(format!("invalid glob pattern '{pattern}': {e}")))?;
        Ok(Matcher::Glob {
            pattern: p,
            opts: MatchOptions {
                case_sensitive,
                require_literal_separator: false,
                require_literal_leading_dot: false,
            },
        })
    } else {
        let needle = if case_sensitive {
            pattern.to_string()
        } else {
            pattern.to_lowercase()
        };
        Ok(Matcher::Substring {
            needle,
            case_sensitive,
        })
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

fn print_output(items: &[FindItem], format: OutputFormat) {
    match format {
        OutputFormat::Json => output::print_json(items),
        OutputFormat::Human => print_human(items),
    }
}

fn print_human(items: &[FindItem]) {
    if items.is_empty() {
        println!("(no matches)");
        return;
    }
    for item in items {
        println!(
            "{}  [{}]  {}",
            item.path,
            common::type_label(&item.kind),
            item.uuid,
        );
    }
}
