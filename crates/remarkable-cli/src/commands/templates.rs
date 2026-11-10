//! Resolve and decode reMarkable paper templates for `render`/`read`.
//!
//! A notebook page records a template *name* (e.g. `"P Lines medium"`) in its
//! `.content`; the matching image lives on the tablet at
//! `/usr/share/remarkable/templates/<name>.svg` (and `.png`).
//! [`TemplateProvider`] resolves a name to a decoded [`BackgroundImage`],
//! trying — in order — a local `--templates-dir`, the on-disk cache, then an
//! on-device fetch over the connection (online mode only), caching whatever it
//! fetches. Results are memoized per `(name, width, height)`, so each template
//! is fetched and decoded at most once per render.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use remarkable_rm::BackgroundImage;
use remarkable_tablet::connection::TabletConnection;

/// On-device directory holding the stock and custom paper templates.
const DEVICE_TEMPLATE_DIR: &str = "/usr/share/remarkable/templates";

/// Resolves page template names to composited backgrounds, with caching.
pub(super) struct TemplateProvider<'a, C> {
    conn: &'a C,
    /// Whether on-device fetch is possible — false for `--from-backup`, whose
    /// connection reads local paths and can't reach `/usr/share/...`.
    allow_remote: bool,
    templates_dir: Option<&'a Path>,
    cache_dir: Option<PathBuf>,
    decoded: HashMap<(String, u32, u32), Option<BackgroundImage>>,
    warned: HashSet<String>,
}

impl<'a, C: TabletConnection> TemplateProvider<'a, C> {
    #[must_use]
    pub(super) fn new(
        conn: &'a C,
        allow_remote: bool,
        templates_dir: Option<&'a Path>,
        cache_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            conn,
            allow_remote,
            templates_dir,
            cache_dir,
            decoded: HashMap::new(),
            warned: HashSet::new(),
        }
    }

    /// The background for a page's template at the given canvas size.
    ///
    /// Returns `None` for blank pages, unnamed templates, or templates that
    /// can't be found or decoded (after emitting a one-time warning) — callers
    /// then render the page on plain white.
    pub(super) async fn background(
        &mut self,
        template: Option<&str>,
        width: u32,
        height: u32,
    ) -> Option<&BackgroundImage> {
        let name = template?;
        if !wants_background(name) {
            return None;
        }
        let key = (name.to_string(), width, height);
        if !self.decoded.contains_key(&key) {
            let decoded = self.resolve(name, width, height).await;
            self.decoded.insert(key.clone(), decoded);
        }
        self.decoded.get(&key).and_then(Option::as_ref)
    }

    async fn resolve(&mut self, name: &str, width: u32, height: u32) -> Option<BackgroundImage> {
        if !is_safe_template_name(name) {
            self.warn(name, "unsafe template name; rendering page on white");
            return None;
        }
        let svg_error = if let Some(svg) = self.variant_bytes(name, "svg").await {
            match remarkable_rm::decode_template(Some(&svg), None, width, height) {
                Ok(background) => return Some(background),
                Err(error) => Some(error),
            }
        } else {
            None
        };
        if let Some(png) = self.variant_bytes(name, "png").await {
            return match remarkable_rm::decode_template(None, Some(&png), width, height) {
                Ok(background) => Some(background),
                Err(error) => {
                    self.warn(
                        name,
                        &format!("decode failed ({error}); rendering page on white"),
                    );
                    None
                }
            };
        }
        if let Some(error) = svg_error {
            self.warn(
                name,
                &format!("decode failed ({error}); rendering page on white"),
            );
        } else {
            self.warn(name, "no template file found; rendering page on white");
        }
        None
    }

    /// Fetch one `<name>.<ext>` variant: `--templates-dir`, then cache, then
    /// (online only) the device, caching what it fetches.
    async fn variant_bytes(&mut self, name: &str, ext: &str) -> Option<Vec<u8>> {
        let filename = format!("{name}.{ext}");

        if let Some(dir) = self.templates_dir
            && let Some(bytes) = read_file_opt(&dir.join(&filename)).await
        {
            return Some(bytes);
        }
        if let Some(dir) = &self.cache_dir
            && let Some(bytes) = read_file_opt(&dir.join(&filename)).await
        {
            return Some(bytes);
        }
        if self.allow_remote {
            let remote = format!("{DEVICE_TEMPLATE_DIR}/{filename}");
            if let Ok(bytes) = self.conn.read_file(&remote).await {
                self.write_cache(&filename, &bytes).await;
                return Some(bytes);
            }
        }
        None
    }

    /// Best-effort cache write — a failure here must never fail the render.
    async fn write_cache(&self, filename: &str, bytes: &[u8]) {
        let Some(dir) = &self.cache_dir else {
            return;
        };
        if tokio::fs::create_dir_all(dir).await.is_ok() {
            let _ = tokio::fs::write(dir.join(filename), bytes).await;
        }
    }

    fn warn(&mut self, name: &str, msg: &str) {
        if self.warned.insert(name.to_string()) {
            eprintln!("warning: template `{name}`: {msg}");
        }
    }
}

async fn read_file_opt(path: &Path) -> Option<Vec<u8>> {
    tokio::fs::read(path).await.ok()
}

/// Whether a template name denotes a real paper background. `"Blank"` (and an
/// empty name) means a plain white page — there is no template image to draw.
fn wants_background(name: &str) -> bool {
    !name.is_empty() && !name.eq_ignore_ascii_case("blank")
}

/// Reject names that aren't a single, safe path component, so a crafted
/// `.content` can't escape the cache / templates directory.
fn is_safe_template_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use remarkable_tablet::connection::FakeConnection;

    const SOLID_RED_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#ff0000"/></svg>"##;

    #[test]
    fn blank_and_empty_want_no_background() {
        assert!(!wants_background("Blank"));
        assert!(!wants_background("blank"));
        assert!(!wants_background(""));
        assert!(wants_background("P Lines medium"));
    }

    #[test]
    fn rejects_path_traversal_names() {
        assert!(is_safe_template_name("P Lines medium"));
        assert!(!is_safe_template_name(".."));
        assert!(!is_safe_template_name("../../etc/passwd"));
        assert!(!is_safe_template_name("a/b"));
        assert!(!is_safe_template_name("a\\b"));
        assert!(!is_safe_template_name(""));
    }

    #[tokio::test]
    async fn fetches_and_caches_svg_without_fetching_png() {
        let conn = FakeConnection::new();
        conn.set_file(
            &format!("{DEVICE_TEMPLATE_DIR}/Test Remote.svg"),
            SOLID_RED_SVG,
        );
        conn.set_file(
            &format!("{DEVICE_TEMPLATE_DIR}/Test Remote.png"),
            b"must not be fetched",
        );
        let cache = tempfile::tempdir().unwrap();
        let mut provider =
            TemplateProvider::new(&conn, true, None, Some(cache.path().to_path_buf()));

        let background = provider
            .background(Some("Test Remote"), 1, 1)
            .await
            .unwrap();

        assert_eq!(background.rgba, [0xff, 0, 0, 0xff]);
        assert_eq!(
            std::fs::read(cache.path().join("Test Remote.svg")).unwrap(),
            SOLID_RED_SVG
        );
        assert!(!cache.path().join("Test Remote.png").exists());
    }
}
