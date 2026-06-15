use std::collections::HashMap;
use std::path::{Path, PathBuf};

use polib::po_file;
use walkdir::WalkDir;

use super::index::{CatalogEntry, CatalogKey, EntryFlags};

// ── Catalog file discovery ────────────────────────────────────────────────────

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

/// Derive `(locale, domain)` from a catalog path.
///
/// - `.pot` → locale `""`, domain = stem
/// - `.po`  → locale = dir two levels up (above `LC_MESSAGES`), domain = stem
/// - `.po` not under `LC_MESSAGES` → `None` (stray file, never pollutes the index)
#[allow(dead_code)]
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

// ── PoLineMap ─────────────────────────────────────────────────────────────────

/// Maps each `CatalogKey` to the 1-based line of its `msgid` in the file.
///
/// polib discards source positions; this scanner recovers them from raw text.
#[allow(dead_code)]
pub struct PoLineMap(HashMap<CatalogKey, u32>);

#[allow(dead_code)]
impl PoLineMap {
    /// Scan the raw `.po` content and build the line map.
    pub fn build(content: &str) -> Self {
        let mut map = HashMap::new();
        let mut iter = content.lines().enumerate().peekable();
        let mut pending_ctxt: Option<String> = None;

        while let Some((i, line)) = iter.next() {
            let lineno = (i + 1) as u32;
            let trimmed = line.trim_start();

            // Blank line resets pending context
            if trimmed.is_empty() {
                pending_ctxt = None;
                continue;
            }

            // msgctxt — collect full (possibly multi-line) value
            if let Some(rest) = trimmed.strip_prefix("msgctxt ") {
                let mut val = read_quoted_first(rest);
                while matches!(iter.peek(), Some((_, l)) if l.trim_start().starts_with('"')) {
                    val.push_str(&read_quoted_first(iter.next().unwrap().1.trim_start()));
                }
                pending_ctxt = if val.is_empty() { None } else { Some(val) };
                continue;
            }

            // msgid — not msgid_plural
            if trimmed.starts_with("msgid ") && !trimmed.starts_with("msgid_plural") {
                let rest = trimmed.trim_start_matches("msgid ").trim_start();
                let msgid_line = lineno;
                let mut msgid = read_quoted_first(rest);
                while matches!(iter.peek(), Some((_, l)) if l.trim_start().starts_with('"')) {
                    msgid.push_str(&read_quoted_first(iter.next().unwrap().1.trim_start()));
                }
                // skip header entry (empty msgid)
                if !msgid.is_empty() {
                    let key = CatalogKey {
                        msgid,
                        msgctxt: pending_ctxt.take(),
                    };
                    map.insert(key, msgid_line);
                } else {
                    pending_ctxt = None;
                }
                continue;
            }
        }

        Self(map)
    }

    /// Look up the 1-based line number for a key.
    pub fn get_line(&self, key: &CatalogKey) -> Option<u32> {
        self.0.get(key).copied()
    }
}

/// Extract and unescape a quoted token: the content of `"..."`.
fn read_quoted_first(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        unescape_po(&s[1..s.len() - 1])
    } else {
        String::new()
    }
}

/// Unescape a `.po` string value (outer quotes already removed).
fn unescape_po(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('"') => result.push('"'),
                Some('\\') => result.push('\\'),
                Some(c) => {
                    result.push('\\');
                    result.push(c);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

// ── load_po_file ──────────────────────────────────────────────────────────────

/// Parse a `.po` or `.pot` file into catalog entries.
///
/// Parse failures return `Err`; the caller logs and skips.
#[allow(dead_code)]
pub fn load_po_file(path: &Path, locale: &str, domain: &str) -> Result<Vec<CatalogEntry>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let catalog = po_file::parse(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let line_map = PoLineMap::build(&content);

    let mut entries = vec![];
    for msg in catalog.messages() {
        let msgid = msg.msgid().to_string();
        if msgid.is_empty() {
            continue; // skip header
        }
        let msgctxt = msg.msgctxt().map(str::to_string);
        let key = CatalogKey {
            msgid: msgid.clone(),
            msgctxt: msgctxt.clone(),
        };
        let line = line_map.get_line(&key).unwrap_or(0);

        let msgid_plural = if msg.is_plural() {
            msg.msgid_plural().ok().map(str::to_string)
        } else {
            None
        };
        let msgstr = if msg.is_plural() {
            msg.msgstr_plural().cloned().unwrap_or_default()
        } else {
            match msg.msgstr() {
                Ok(s) => vec![s.to_string()],
                Err(_) => vec![],
            }
        };

        entries.push(CatalogEntry {
            locale: locale.to_string(),
            domain: domain.to_string(),
            msgid,
            msgctxt,
            msgid_plural,
            msgstr,
            flags: EntryFlags {
                fuzzy: msg.is_fuzzy(),
                obsolete: false,
            },
            file_path: path.to_path_buf(),
            line,
        });
    }
    Ok(entries)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

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

    fn write_po(dir: &TempDir, rel: &str, content: &str) -> PathBuf {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
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
        let found = discover_catalogs(&[dir1.path().to_path_buf(), dir2.path().to_path_buf()]);
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
        assert_eq!(
            locale_domain_from_po_path(Path::new("/locale/de/LC_MESSAGES/messages.po")),
            Some(("de".to_string(), "messages".to_string()))
        );
    }

    #[test]
    fn pot_path_gives_empty_locale() {
        assert_eq!(
            locale_domain_from_po_path(Path::new("/locale/messages.pot")),
            Some(("".to_string(), "messages".to_string()))
        );
    }

    #[test]
    fn po_not_under_lc_messages_returns_none() {
        assert!(locale_domain_from_po_path(Path::new("/locale/de/messages.po")).is_none());
    }

    #[test]
    fn unknown_extension_returns_none() {
        assert!(
            locale_domain_from_po_path(Path::new("/locale/de/LC_MESSAGES/messages.txt")).is_none()
        );
    }

    #[test]
    fn multi_domain_po_extracts_correct_domain() {
        assert_eq!(
            locale_domain_from_po_path(Path::new("/locale/fr/LC_MESSAGES/admin.po")),
            Some(("fr".to_string(), "admin".to_string()))
        );
    }

    // --- PoLineMap ---

    #[test]
    fn line_map_simple_msgid() {
        let content = concat!(
            "msgid \"\"\n",
            "msgstr \"\"\n",
            "\n",
            "msgid \"Checkout\"\n",
            "msgstr \"Kasse\"\n",
        );
        let map = PoLineMap::build(content);
        assert_eq!(map.get_line(&CatalogKey::new("Checkout")), Some(4));
    }

    #[test]
    fn line_map_skips_header() {
        let map = PoLineMap::build("msgid \"\"\nmsgstr \"\"\n");
        assert_eq!(map.get_line(&CatalogKey::new("")), None);
    }

    #[test]
    fn line_map_with_msgctxt() {
        let content = concat!(
            "msgid \"\"\nmsgstr \"\"\n\n",
            "msgctxt \"button\"\n",
            "msgid \"Save\"\n",
            "msgstr \"Speichern\"\n",
        );
        let map = PoLineMap::build(content);
        assert_eq!(
            map.get_line(&CatalogKey::with_ctx("Save", "button")),
            Some(5)
        );
    }

    #[test]
    fn line_map_multiple_entries() {
        let content = concat!(
            "msgid \"\"\nmsgstr \"\"\n\n",
            "msgid \"Alpha\"\nmsgstr \"\"\n\n",
            "msgid \"Beta\"\nmsgstr \"\"\n",
        );
        let map = PoLineMap::build(content);
        assert_eq!(map.get_line(&CatalogKey::new("Alpha")), Some(4));
        assert_eq!(map.get_line(&CatalogKey::new("Beta")), Some(7));
    }

    #[test]
    fn line_map_multiline_msgid() {
        let content = concat!(
            "msgid \"\"\nmsgstr \"\"\n\n",
            "msgid \"\"\n",
            "\"Hello \"\n",
            "\"World\"\n",
            "msgstr \"\"\n",
        );
        let map = PoLineMap::build(content);
        assert_eq!(map.get_line(&CatalogKey::new("Hello World")), Some(4));
    }

    // --- load_po_file ---

    const MINIMAL_PO: &str = concat!(
        "msgid \"\"\n",
        "msgstr \"\"\n",
        "\"Content-Type: text/plain; charset=UTF-8\\n\"\n",
        "\n",
        "msgid \"Checkout\"\n",
        "msgstr \"Kasse\"\n",
        "\n",
        "msgid \"Save\"\n",
        "msgstr \"\"\n",
    );

    #[test]
    fn load_po_returns_entries_without_header() {
        let dir = TempDir::new().unwrap();
        let path = write_po(&dir, "de/LC_MESSAGES/messages.po", MINIMAL_PO);
        let entries = load_po_file(&path, "de", "messages").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| !e.msgid.is_empty()));
    }

    #[test]
    fn load_po_entry_has_correct_locale_domain() {
        let dir = TempDir::new().unwrap();
        let path = write_po(&dir, "de/LC_MESSAGES/messages.po", MINIMAL_PO);
        let entries = load_po_file(&path, "de", "messages").unwrap();
        assert!(
            entries
                .iter()
                .all(|e| e.locale == "de" && e.domain == "messages")
        );
    }

    #[test]
    fn load_po_entry_has_line_number() {
        let dir = TempDir::new().unwrap();
        let path = write_po(&dir, "de/LC_MESSAGES/messages.po", MINIMAL_PO);
        let entries = load_po_file(&path, "de", "messages").unwrap();
        let checkout = entries.iter().find(|e| e.msgid == "Checkout").unwrap();
        assert_eq!(checkout.line, 5);
    }

    #[test]
    fn load_po_fuzzy_entry() {
        let content = concat!(
            "msgid \"\"\nmsgstr \"\"\n",
            "\"Content-Type: text/plain; charset=UTF-8\\n\"\n\n",
            "#, fuzzy\n",
            "msgid \"Fuzzy msg\"\n",
            "msgstr \"Unvollständig\"\n",
        );
        let dir = TempDir::new().unwrap();
        let path = write_po(&dir, "de/LC_MESSAGES/messages.po", content);
        let entries = load_po_file(&path, "de", "messages").unwrap();
        let entry = entries.iter().find(|e| e.msgid == "Fuzzy msg").unwrap();
        assert!(entry.flags.fuzzy);
    }

    #[test]
    fn load_po_unreadable_returns_err() {
        let result = load_po_file(Path::new("/nonexistent/messages.po"), "de", "messages");
        assert!(result.is_err());
    }
}
