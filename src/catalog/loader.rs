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
                    map.entry(key).or_insert(msgid_line);
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

// ── load_po_file / load_po_from_str ──────────────────────────────────────────

/// Parse a `.po` or `.pot` file into catalog entries.
///
/// Parse failures return `Err`; the caller logs and skips.
#[allow(dead_code)]
pub fn load_po_file(path: &Path, locale: &str, domain: &str) -> Result<Vec<CatalogEntry>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let line_map = PoLineMap::build(&content);
    let catalog = parse_po_str(&content, path)?;
    parse_catalog(catalog, &line_map, &content, path, locale, domain)
}

/// Parse `.po` content from an in-memory string into catalog entries.
///
/// `original_path` is stored in the resulting entries' `file_path` field;
/// it does not need to exist on disk.
#[allow(dead_code)]
pub fn load_po_from_str(
    content: &str,
    original_path: &Path,
    locale: &str,
    domain: &str,
) -> Result<Vec<CatalogEntry>, String> {
    let line_map = PoLineMap::build(content);
    let catalog = parse_po_str(content, original_path)?;
    parse_catalog(catalog, &line_map, content, original_path, locale, domain)
}

/// Parse PO content from a string, tolerating a missing or ill-formed header by
/// synthesizing a minimal one and retrying.
fn parse_po_str(content: &str, original_path: &Path) -> Result<polib::catalog::Catalog, String> {
    let cursor = std::io::Cursor::new(content.as_bytes());
    match po_file::parse_from_reader(cursor) {
        Ok(catalog) => Ok(catalog),
        Err(e)
            if e.to_string()
                .contains("Metadata does not exist or is ill-formed") =>
        {
            let patched = format!(
                "msgid \"\"\nmsgstr \"Content-Type: text/plain; charset=UTF-8\\n\"\n\n{content}"
            );
            let cursor = std::io::Cursor::new(patched.as_bytes());
            po_file::parse_from_reader(cursor)
                .map_err(|e2| format!("{}: {e2}", original_path.display()))
        }
        Err(e) => Err(format!("{}: {e}", original_path.display())),
    }
}

/// Internal: convert a parsed polib `Catalog` into `CatalogEntry` values.
///
/// The `content` parameter is the raw PO text, used to recover information
/// that polib discards: duplicate msgids, obsolete entries, and the header.
fn parse_catalog(
    catalog: polib::catalog::Catalog,
    line_map: &PoLineMap,
    content: &str,
    original_path: &Path,
    locale: &str,
    domain: &str,
) -> Result<Vec<CatalogEntry>, String> {
    let mut entries = vec![];

    // Synthesize a header CatalogEntry from the parsed metadata so that
    // check_catalog can find the nplurals declaration (po/plural-count) and the
    // Content-Type charset (po/header-missing).  polib stores metadata
    // separately and never exposes it as a message, so we reconstruct the
    // header msgstr from the metadata fields that check_catalog reads.
    //
    // Only synthesize the header if the original file actually had one — if
    // parse_po_str had to patch a minimal header in (because the file was
    // headerless), we must NOT emit a synthetic header entry, otherwise
    // po/header-missing would never fire for headerless catalogs.
    if raw_content_has_header(content) {
        let header_msgstr = build_header_msgstr(&catalog.metadata);
        if !header_msgstr.is_empty() {
            entries.push(CatalogEntry {
                locale: locale.to_string(),
                domain: domain.to_string(),
                msgid: String::new(),
                msgctxt: None,
                msgid_plural: None,
                msgstr: vec![header_msgstr],
                flags: EntryFlags {
                    fuzzy: false,
                    obsolete: false,
                },
                file_path: original_path.to_path_buf(),
                line: 1,
            });
        }
    }

    // Detect duplicate msgids using a raw text scan.  polib deduplicates via
    // append_or_update, so the catalog only holds the last occurrence.  We
    // recover the first-seen line per key and flag any second occurrence as a
    // duplicate entry with the same msgstr as the polib-surviving entry.
    let dup_keys = scan_duplicate_msgids(content);

    for msg in catalog.messages() {
        let msgid = msg.msgid().to_string();
        if msgid.is_empty() {
            continue;
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
            msgid: msgid.clone(),
            msgctxt: msgctxt.clone(),
            msgid_plural: msgid_plural.clone(),
            msgstr: msgstr.clone(),
            flags: EntryFlags {
                fuzzy: msg.is_fuzzy(),
                obsolete: false,
            },
            file_path: original_path.to_path_buf(),
            line,
        });

        // If the key appears more than once in the raw text, inject a duplicate
        // entry so that po/duplicate-id can fire on the second occurrence.
        if let Some(&dup_line) = dup_keys.get(&key) {
            if dup_line != line {
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
                    file_path: original_path.to_path_buf(),
                    line: dup_line,
                });
            }
        }
    }

    // Recover obsolete entries (#~ msgid) which polib silently drops.
    let obsolete = scan_obsolete_entries(content, original_path, locale, domain);
    entries.extend(obsolete);

    Ok(entries)
}

/// Return true if the raw PO content contains a header entry (`msgid ""`).
///
/// A header is present when the file opens with `msgid ""` (possibly preceded
/// by comment lines) followed by a non-empty `msgstr`.  We detect this with a
/// simple scan rather than a full parse so we can distinguish a genuine header
/// from a file that was patched by parse_po_str.
fn raw_content_has_header(content: &str) -> bool {
    let mut saw_empty_msgid = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            // If we already committed to a non-header first entry, stop.
            if saw_empty_msgid {
                break;
            }
            continue;
        }
        if trimmed == "msgid \"\"" || trimmed.starts_with("msgid \"\"") {
            saw_empty_msgid = true;
            continue;
        }
        if saw_empty_msgid && trimmed.starts_with("msgstr") {
            // The empty msgid is followed by msgstr — this is the header.
            return true;
        }
        // Non-comment, non-empty-msgid line before we saw the header — no header.
        if !saw_empty_msgid {
            return false;
        }
    }
    false
}

/// Reconstruct the header msgstr from polib's parsed metadata.
///
/// check_catalog calls parse_nplurals() on the concatenated msgstr of the
/// header entry.  We need to emit the fields it looks for.
fn build_header_msgstr(metadata: &polib::metadata::CatalogMetadata) -> String {
    let mut parts = Vec::new();
    if !metadata.content_type.is_empty() {
        parts.push(format!("Content-Type: {}\n", metadata.content_type));
    }
    let plural_str = metadata.plural_rules.dump();
    // Always include Plural-Forms; check_catalog uses it for po/plural-count.
    parts.push(format!("Plural-Forms: {}\n", plural_str));
    parts.concat()
}

/// Scan raw PO content and return a map of keys whose msgid appears more than
/// once.  The value is the 1-based line of the *second* occurrence so that
/// check_catalog can place the duplicate finding there.
fn scan_duplicate_msgids(content: &str) -> HashMap<CatalogKey, u32> {
    let mut first_seen: HashMap<CatalogKey, u32> = HashMap::new();
    let mut duplicates: HashMap<CatalogKey, u32> = HashMap::new();

    let mut iter = content.lines().enumerate().peekable();
    let mut pending_ctxt: Option<String> = None;
    let mut in_obsolete = false;

    while let Some((i, line)) = iter.next() {
        let lineno = (i + 1) as u32;
        let trimmed = line.trim_start();

        if trimmed.is_empty() {
            pending_ctxt = None;
            in_obsolete = false;
            continue;
        }

        // Skip obsolete entries
        if trimmed.starts_with("#~") {
            in_obsolete = true;
            continue;
        }
        if in_obsolete {
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("msgctxt ") {
            let mut val = read_quoted_first(rest);
            while matches!(iter.peek(), Some((_, l)) if l.trim_start().starts_with('"')) {
                val.push_str(&read_quoted_first(iter.next().unwrap().1.trim_start()));
            }
            pending_ctxt = if val.is_empty() { None } else { Some(val) };
            continue;
        }

        if trimmed.starts_with("msgid ") && !trimmed.starts_with("msgid_plural") {
            let rest = trimmed.trim_start_matches("msgid ").trim_start();
            let msgid_line = lineno;
            let mut msgid = read_quoted_first(rest);
            while matches!(iter.peek(), Some((_, l)) if l.trim_start().starts_with('"')) {
                msgid.push_str(&read_quoted_first(iter.next().unwrap().1.trim_start()));
            }
            if msgid.is_empty() {
                pending_ctxt = None;
                continue;
            }
            let key = CatalogKey {
                msgid,
                msgctxt: pending_ctxt.take(),
            };
            if let Some(_first_line) = first_seen.get(&key) {
                duplicates.entry(key).or_insert(msgid_line);
            } else {
                first_seen.insert(key, msgid_line);
            }
        }
    }

    duplicates
}

/// Scan raw PO content for obsolete entries (`#~ msgid ...`) and return them
/// as CatalogEntry values with `flags.obsolete = true`.
///
/// polib silently discards `#~` lines, so we recover them here.
fn scan_obsolete_entries(
    content: &str,
    original_path: &Path,
    locale: &str,
    domain: &str,
) -> Vec<CatalogEntry> {
    let mut result = Vec::new();
    let mut iter = content.lines().enumerate().peekable();

    while let Some((i, line)) = iter.next() {
        let lineno = (i + 1) as u32;
        let trimmed = line.trim_start();

        // Obsolete msgid line
        let rest = if let Some(r) = trimmed.strip_prefix("#~ msgid ") {
            r
        } else if let Some(r) = trimmed.strip_prefix("#~msgid ") {
            r
        } else {
            continue;
        };

        if rest.trim_start().starts_with("\"\"") {
            continue; // skip the obsolete header sentinel
        }

        let mut msgid = read_quoted_first(rest.trim_start());
        // Collect continuation lines (#~ "...")
        while matches!(iter.peek(), Some((_, l)) if {
            let t = l.trim_start();
            t.starts_with("#~ \"") || t.starts_with("#~\"")
        }) {
            let cont = iter.next().unwrap().1.trim_start().to_string();
            let cont = cont.trim_start_matches("#~").trim_start();
            msgid.push_str(&read_quoted_first(cont));
        }

        if msgid.is_empty() {
            continue;
        }

        result.push(CatalogEntry {
            locale: locale.to_string(),
            domain: domain.to_string(),
            msgid,
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec![String::new()],
            flags: EntryFlags {
                fuzzy: false,
                obsolete: true,
            },
            file_path: original_path.to_path_buf(),
            line: lineno,
        });
    }

    result
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
    fn line_map_duplicate_msgid_keeps_first() {
        // A file with a duplicated msgid: the line map must record the FIRST occurrence.
        let content = concat!(
            "msgid \"\"\nmsgstr \"\"\n\n",
            "msgid \"Duplicate\"\n",
            "msgstr \"First\"\n",
            "\n",
            "msgid \"Duplicate\"\n",
            "msgstr \"Second\"\n",
        );
        let map = PoLineMap::build(content);
        // Line 4 is the first occurrence; line 7 is the duplicate.
        assert_eq!(map.get_line(&CatalogKey::new("Duplicate")), Some(4));
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

    #[test]
    fn line_map_pot_entry_with_msgctxt() {
        // A .pot file may contain entries with msgctxt; the line map must
        // record them under the combined key so load_po_from_str can look them up.
        let content = concat!(
            "msgid \"\"\nmsgstr \"\"\n\n",
            "msgctxt \"button\"\n",
            "msgid \"Submit\"\n",
            "msgstr \"\"\n",
        );
        let map = PoLineMap::build(content);
        assert_eq!(
            map.get_line(&CatalogKey::with_ctx("Submit", "button")),
            Some(5)
        );
        // The same msgid without context must NOT match.
        assert_eq!(map.get_line(&CatalogKey::new("Submit")), None);
    }

    #[test]
    fn line_map_skips_obsolete_entries() {
        // Obsolete entries start with "#~"; the scanner must ignore them entirely.
        let content = concat!(
            "msgid \"\"\nmsgstr \"\"\n\n",
            "#~ msgid \"OldMsg\"\n",
            "#~ msgstr \"AltesMsg\"\n",
            "\n",
            "msgid \"NewMsg\"\n",
            "msgstr \"NeueMsg\"\n",
        );
        let map = PoLineMap::build(content);
        assert_eq!(map.get_line(&CatalogKey::new("OldMsg")), None);
        assert_eq!(map.get_line(&CatalogKey::new("NewMsg")), Some(7));
    }

    #[test]
    fn line_map_plural_entry_maps_msgid_line_not_plural_line() {
        // PoLineMap must record the line of the `msgid` keyword, not `msgid_plural`.
        let content = concat!(
            "msgid \"\"\nmsgstr \"\"\n\n",
            "msgid \"%(n)d item\"\n", // line 4
            "msgid_plural \"%(n)d items\"\n",
            "msgstr[0] \"\"\n",
            "msgstr[1] \"\"\n",
        );
        let map = PoLineMap::build(content);
        assert_eq!(map.get_line(&CatalogKey::new("%(n)d item")), Some(4));
        // msgid_plural value must not be registered as a separate key.
        assert_eq!(map.get_line(&CatalogKey::new("%(n)d items")), None);
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
        // MINIMAL_PO has 2 non-header msgids plus 1 synthesized header entry.
        assert_eq!(entries.len(), 3);
        // At least 2 entries have non-empty msgids (the real translations).
        assert_eq!(entries.iter().filter(|e| !e.msgid.is_empty()).count(), 2);
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

    // --- load_po_from_str ---

    #[test]
    fn load_po_from_str_parses_buffer_content() {
        let content = concat!(
            "msgid \"\"\n",
            "msgstr \"\"\n",
            "\"Content-Type: text/plain; charset=UTF-8\\n\"\n",
            "\n",
            "msgid \"Buffer Entry\"\n",
            "msgstr \"Puffer Eintrag\"\n",
        );
        let path = Path::new("/locale/de/LC_MESSAGES/messages.po");
        let entries = load_po_from_str(content, path, "de", "messages").unwrap();
        // The synthesized header plus one real msgid.
        assert_eq!(entries.len(), 2);
        let entry = entries.iter().find(|e| e.msgid == "Buffer Entry").unwrap();
        assert_eq!(entry.locale, "de");
        assert_eq!(entry.domain, "messages");
        assert_eq!(entry.file_path, path);
    }

    #[test]
    fn load_po_from_str_has_line_number() {
        let content = concat!(
            "msgid \"\"\n",
            "msgstr \"\"\n",
            "\"Content-Type: text/plain; charset=UTF-8\\n\"\n",
            "\n",
            "msgid \"Alpha\"\n",
            "msgstr \"\"\n",
        );
        let path = Path::new("/locale/de/LC_MESSAGES/messages.po");
        let entries = load_po_from_str(content, path, "de", "messages").unwrap();
        let alpha = entries.iter().find(|e| e.msgid == "Alpha").unwrap();
        assert_eq!(alpha.line, 5);
    }

    #[test]
    fn load_po_from_str_file_path_is_original_not_tempfile() {
        let content = concat!(
            "msgid \"\"\nmsgstr \"\"\n",
            "\"Content-Type: text/plain; charset=UTF-8\\n\"\n\n",
            "msgid \"Msg\"\nmsgstr \"\"\n",
        );
        let original = Path::new("/some/virtual/path.po");
        let entries = load_po_from_str(content, original, "fr", "admin").unwrap();
        assert_eq!(entries[0].file_path, original);
    }

    #[test]
    fn load_po_headerless_file_yields_entries() {
        let content = concat!(
            "msgid \"First\"\n",
            "msgstr \"Erstes\"\n",
            "\n",
            "msgid \"Second\"\n",
            "msgstr \"\"\n",
        );
        let dir = TempDir::new().unwrap();
        let path = write_po(&dir, "de/LC_MESSAGES/messages.po", content);
        let entries = load_po_file(&path, "de", "messages").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.msgid == "First"));
        assert!(entries.iter().any(|e| e.msgid == "Second"));
    }

    #[test]
    fn load_po_from_str_headerless_yields_entries() {
        let content = concat!(
            "msgid \"Alpha\"\n",
            "msgstr \"Alfa\"\n",
            "\n",
            "msgid \"Beta\"\n",
            "msgstr \"\"\n",
        );
        let path = Path::new("/locale/de/LC_MESSAGES/messages.po");
        let entries = load_po_from_str(content, path, "de", "messages").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.msgid == "Alpha"));
        assert!(entries.iter().any(|e| e.msgid == "Beta"));
    }

    #[test]
    fn load_po_from_str_disk_differs_from_buffer() {
        let dir = TempDir::new().unwrap();
        let path = write_po(&dir, "de/LC_MESSAGES/messages.po", MINIMAL_PO);

        let disk_entries = load_po_file(&path, "de", "messages").unwrap();

        let buffer_content = concat!(
            "msgid \"\"\nmsgstr \"\"\n",
            "\"Content-Type: text/plain; charset=UTF-8\\n\"\n\n",
            "msgid \"Only In Buffer\"\nmsgstr \"\"\n",
        );
        let buf_entries = load_po_from_str(buffer_content, &path, "de", "messages").unwrap();

        let disk_ids: Vec<_> = disk_entries.iter().map(|e| e.msgid.as_str()).collect();
        let buf_ids: Vec<_> = buf_entries.iter().map(|e| e.msgid.as_str()).collect();
        assert!(disk_ids.contains(&"Checkout"));
        assert!(!disk_ids.contains(&"Only In Buffer"));
        assert!(buf_ids.contains(&"Only In Buffer"));
        assert!(!buf_ids.contains(&"Checkout"));
    }
}
