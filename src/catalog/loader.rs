use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Walk `locale_dirs` and collect every `.po` and `.pot` file found recursively.
#[allow(dead_code)]
pub fn discover_catalogs(locale_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = vec![];
    for dir in locale_dirs {
        for entry in WalkDir::new(dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let ext = entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if ext == "po" || ext == "pot" {
                paths.push(entry.into_path());
            }
        }
    }
    paths
}

#[allow(dead_code)]
/// Derive `(locale, domain)` from a catalog path.
///
/// - `.pot` → locale `""`, domain = stem
/// - `.po`  → locale = dir two levels up (above `LC_MESSAGES`), domain = stem
/// - `.po` not under `LC_MESSAGES` → `None` (stray file, never pollutes the index)
pub fn locale_domain_from_po_path(path: &Path) -> Option<(String, String)> {
    let ext = path.extension()?.to_str()?;
    let domain = path.file_stem()?.to_str()?.to_string();
    match ext {
        "pot" => Some((String::new(), domain)),
        "po" => {
            let parent = path.parent()?;
            if parent.file_name()?.to_str()? != "LC_MESSAGES" {
                return None;
            }
            let locale = parent.parent()?.file_name()?.to_str()?.to_string();
            Some((locale, domain))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mk(dir: &TempDir, rel: &str) -> PathBuf {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "").unwrap();
        p
    }

    // --- discover_catalogs ---

    #[test]
    fn discover_finds_po_and_pot() {
        let dir = TempDir::new().unwrap();
        mk(&dir, "de/LC_MESSAGES/messages.po");
        mk(&dir, "messages.pot");
        mk(&dir, "views.py");
        let found = discover_catalogs(&[dir.path().to_path_buf()]);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|p| p.extension().unwrap() == "po"));
        assert!(found.iter().any(|p| p.extension().unwrap() == "pot"));
    }

    #[test]
    fn discover_skips_non_catalog_files() {
        let dir = TempDir::new().unwrap();
        mk(&dir, "views.py");
        mk(&dir, "README.md");
        let found = discover_catalogs(&[dir.path().to_path_buf()]);
        assert!(found.is_empty());
    }

    #[test]
    fn discover_walks_subdirectories() {
        let dir = TempDir::new().unwrap();
        mk(&dir, "locale/de/LC_MESSAGES/messages.po");
        mk(&dir, "locale/fr/LC_MESSAGES/messages.po");
        let found = discover_catalogs(&[dir.path().to_path_buf()]);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn discover_over_multiple_dirs() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();
        mk(&dir1, "de/LC_MESSAGES/messages.po");
        mk(&dir2, "fr/LC_MESSAGES/messages.po");
        let found =
            discover_catalogs(&[dir1.path().to_path_buf(), dir2.path().to_path_buf()]);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn discover_empty_dirs_returns_empty() {
        let dir = TempDir::new().unwrap();
        let found = discover_catalogs(&[dir.path().to_path_buf()]);
        assert!(found.is_empty());
    }

    // --- locale_domain_from_po_path ---

    #[test]
    fn po_path_extracts_locale_and_domain() {
        let path = Path::new("/locale/de/LC_MESSAGES/messages.po");
        assert_eq!(
            locale_domain_from_po_path(path),
            Some(("de".to_string(), "messages".to_string()))
        );
    }

    #[test]
    fn pot_path_gives_empty_locale() {
        let path = Path::new("/locale/messages.pot");
        assert_eq!(
            locale_domain_from_po_path(path),
            Some(("".to_string(), "messages".to_string()))
        );
    }

    #[test]
    fn po_not_under_lc_messages_returns_none() {
        let path = Path::new("/locale/de/messages.po");
        assert!(locale_domain_from_po_path(path).is_none());
    }

    #[test]
    fn unknown_extension_returns_none() {
        let path = Path::new("/locale/de/LC_MESSAGES/messages.txt");
        assert!(locale_domain_from_po_path(path).is_none());
    }

    #[test]
    fn multi_domain_po_extracts_correct_domain() {
        let path = Path::new("/locale/fr/LC_MESSAGES/admin.po");
        assert_eq!(
            locale_domain_from_po_path(path),
            Some(("fr".to_string(), "admin".to_string()))
        );
    }
}
