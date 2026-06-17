use std::collections::HashMap;

use tower_lsp_server::ls_types::{Position, Range, TextEdit, Uri, WorkspaceEdit};

use crate::catalog::index::{CatalogEntry, CatalogIndex, CatalogKey};
use crate::extract::types::TranslationCall;
use crate::util::po_edit::{escape_po, msgid_block_range, parse_entry_spans, span_at_line};

/// Resolve the msgid range and key at `pos` in a source file.
///
/// The cursor must sit inside a call's `msgid_range`; returns `None` for
/// non-literal msgids or any position that does not lie on a known call (REQ-RNM-02).
pub fn prepare_rename_source(
    calls: &[TranslationCall],
    pos: Position,
) -> Option<(Range, CatalogKey)> {
    let call = calls.iter().find(|c| {
        c.msgid.is_some()
            && c.msgid_range
                .is_some_and(|r| crate::util::pos_in_range(pos, r))
    })?;
    let msgid = call.msgid.clone()?;
    let range = call.msgid_range?;
    let key = CatalogKey {
        msgid,
        msgctxt: call.msgctxt.clone(),
    };
    Some((range, key))
}

/// Resolve the msgid range and key at `pos` in a catalog file.
///
/// The cursor must sit on the `msgid` keyword line of a real (non-header)
/// entry; returns `None` otherwise (REQ-RNM-01, REQ-RNM-02).
pub fn prepare_rename_catalog(
    entries: &[&CatalogEntry],
    content_lines: &[&str],
    pos: Position,
) -> Option<(Range, CatalogKey)> {
    let cursor_line = pos.line;
    let entry = entries
        .iter()
        .find(|e| e.line > 0 && e.line - 1 == cursor_line && !e.msgid.is_empty())?;

    let line = content_lines.get(cursor_line as usize)?;
    let keyword_pos = line.find("msgid \"")?; // handles `#~ msgid "..."` too
    let start = (keyword_pos + 7) as u32;
    let end_quote = line.rfind('"').filter(|&i| i >= keyword_pos + 7)?;
    let end = end_quote as u32;

    if start >= end {
        return None; // zero-width or inverted range — e.g. multi-line msgid whose first line is `msgid ""`
    }

    let range = Range {
        start: Position {
            line: cursor_line,
            character: start,
        },
        end: Position {
            line: cursor_line,
            character: end,
        },
    };
    let key = CatalogKey {
        msgid: entry.msgid.clone(),
        msgctxt: entry.msgctxt.clone(),
    };
    Some((range, key))
}

/// Build a `WorkspaceEdit` that renames every occurrence of `key.msgid` to
/// `new_name` across all catalog entries and source call sites.
///
/// Returns `Err(message)` when renaming would merge two messages (REQ-RNM-07).
///
/// `catalog_bufs` is a map from catalog file path to buffer content (open
/// buffer or on-disk text); entries whose path is absent are skipped.
pub fn build_rename_edit(
    key: &CatalogKey,
    new_name: &str,
    index: &CatalogIndex,
    catalog_bufs: &HashMap<std::path::PathBuf, String>,
    call_sites: &[(Uri, Vec<TranslationCall>)],
) -> Result<WorkspaceEdit, String> {
    // REQ-RNM-07: collision guard — refuse if the target key already exists in catalog.
    let new_key = CatalogKey {
        msgid: new_name.into(),
        msgctxt: key.msgctxt.clone(),
    };
    if !index.lookup(&new_key).is_empty() || index.lookup_pot(&new_key).is_some() {
        return Err(format!(
            "Cannot rename: a message '{}' already exists — renaming would merge two messages",
            new_name
        ));
    }

    // Also check source call sites — if any call site already uses new_name as its
    // msgid, renaming would create a collision in the source (REQ-RNM-07).
    for (_uri, calls) in call_sites {
        for call in calls {
            if call.msgid.as_deref() == Some(new_name)
                && call.msgctxt.as_deref() == key.msgctxt.as_deref()
                && call.msgid.as_deref() != Some(key.msgid.as_str())
            {
                return Err(format!(
                    "Cannot rename: a call site already uses '{}' as a msgid — renaming would create a collision",
                    new_name
                ));
            }
        }
    }

    let escaped = escape_po(new_name);
    let new_msgid_line = format!("msgid \"{escaped}\"\n");
    let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();

    // ── Catalog edits (REQ-RNM-03) ───────────────────────────────────────────

    let mut catalog_entries: Vec<&CatalogEntry> = index.lookup(key).into_iter().collect();
    if let Some(pot) = index.lookup_pot(key) {
        catalog_entries.push(pot);
    }

    // Deduplicate by file path so a malformed index with duplicate keys does
    // not emit two edits for the same msgid line.
    let mut seen_paths: std::collections::HashSet<&std::path::Path> =
        std::collections::HashSet::new();
    for entry in catalog_entries {
        if !seen_paths.insert(entry.file_path.as_path()) {
            continue;
        }
        let Some(content) = catalog_bufs.get(&entry.file_path) else {
            continue;
        };
        let Some(uri) = Uri::from_file_path(&entry.file_path) else {
            continue;
        };
        let msgid_line = entry.line.saturating_sub(1);
        let spans = parse_entry_spans(content);
        let text_edit = if let Some(span) = span_at_line(&spans, msgid_line) {
            // Replace the entire msgid block (handles multi-line msgids).
            let range = msgid_block_range(span);
            TextEdit {
                range,
                new_text: new_msgid_line.clone(),
            }
        } else {
            // Fallback: replace just the quoted text on the msgid line.
            let lines: Vec<&str> = content.lines().collect();
            let Some(line_content) = lines.get(msgid_line as usize) else {
                continue;
            };
            let Some(kw_pos) = line_content.find("msgid \"") else {
                continue;
            };
            let start_char = (kw_pos + 7) as u32;
            let end_char = line_content.chars().count() as u32 - 1;
            let range = Range {
                start: Position {
                    line: msgid_line,
                    character: start_char,
                },
                end: Position {
                    line: msgid_line,
                    character: end_char,
                },
            };
            TextEdit {
                range,
                new_text: escaped.clone(),
            }
        };
        changes.entry(uri).or_default().push(text_edit);
    }

    // ── Source edits (REQ-RNM-04) ────────────────────────────────────────────

    for (uri, calls) in call_sites {
        for call in calls {
            if call.msgid.as_deref() == Some(key.msgid.as_str())
                && call.msgctxt.as_deref() == key.msgctxt.as_deref()
            {
                if let Some(range) = call.msgid_range {
                    changes.entry(uri.clone()).or_default().push(TextEdit {
                        range,
                        new_text: new_name.to_string(),
                    });
                }
            }
        }
    }

    Ok(WorkspaceEdit {
        changes: Some(changes),
        ..WorkspaceEdit::default()
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::catalog::index::{CatalogEntry, EntryFlags};
    use crate::extract::python;

    fn entry(msgid: &str, path: &str, line: u32) -> CatalogEntry {
        CatalogEntry {
            locale: "de".into(),
            domain: "messages".into(),
            msgid: msgid.into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec!["Übersetzung".into()],
            flags: EntryFlags {
                fuzzy: false,
                obsolete: false,
            },
            file_path: PathBuf::from(path),
            line,
        }
    }

    fn calls(src: &str) -> Vec<TranslationCall> {
        python::extract(src.as_bytes(), &HashMap::new())
    }

    fn pos(line: u32, ch: u32) -> Position {
        Position {
            line,
            character: ch,
        }
    }

    fn uri(path: &str) -> Uri {
        Uri::from_file_path(path).unwrap()
    }

    // ── REQ-RNM-01: prepare returns msgid range ───────────────────────────────

    #[test]
    fn req_rnm_01_prepare_source_returns_msgid_range() {
        let src = r#"_("Checkout")"#;
        let c = calls(src);
        let (range, key) = prepare_rename_source(&c, pos(0, 3)).unwrap();
        assert_eq!(key.msgid, "Checkout");
        // The range should cover the `Checkout` literal (no quotes).
        assert_eq!(range.start.line, 0);
    }

    #[test]
    fn req_rnm_01_prepare_catalog_returns_msgid_range() {
        let content = "msgid \"Checkout\"\nmsgstr \"Kasse\"\n";
        let lines: Vec<&str> = content.lines().collect();
        let e = entry("Checkout", "/locale/de/messages.po", 1);
        let (range, key) = prepare_rename_catalog(&[&e], &lines, pos(0, 8)).unwrap();
        assert_eq!(key.msgid, "Checkout");
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 7); // after `msgid "`
        assert_eq!(range.end.character, 15); // before closing `"`
    }

    // ── REQ-RNM-02: non-renameable positions are rejected ────────────────────

    #[test]
    fn req_rnm_02_prepare_source_rejects_non_literal() {
        let src = r#"_(f"Hi {user}")"#;
        let c = calls(src);
        let result = prepare_rename_source(&c, pos(0, 3));
        assert!(result.is_none());
    }

    #[test]
    fn req_rnm_02_prepare_catalog_rejects_header_entry() {
        let content = "msgid \"\"\nmsgstr \"Content-Type: text/plain\"\n";
        let lines: Vec<&str> = content.lines().collect();
        let mut e = entry("", "/locale/de/messages.po", 1);
        e.msgid = "".into();
        let result = prepare_rename_catalog(&[&e], &lines, pos(0, 0));
        assert!(result.is_none());
    }

    #[test]
    fn req_rnm_02_prepare_source_rejects_cursor_outside_calls() {
        let src = r#"x = 1"#;
        let c = calls(src);
        let result = prepare_rename_source(&c, pos(0, 2));
        assert!(result.is_none());
    }

    // ── REQ-RNM-03: catalog edits ────────────────────────────────────────────

    #[test]
    fn req_rnm_03_builds_workspace_edit_for_catalog_entry() {
        let content = "msgid \"Checkout\"\nmsgstr \"Kasse\"\n";
        let e = entry("Checkout", "/locale/de/messages.po", 1);
        let index = CatalogIndex::build(vec![e]);
        let key = CatalogKey::new("Checkout");
        let mut bufs = HashMap::new();
        bufs.insert(PathBuf::from("/locale/de/messages.po"), content.to_string());
        let edit = build_rename_edit(&key, "Checkout page", &index, &bufs, &[]).unwrap();
        let changes = edit.changes.unwrap();
        let edits = changes.values().next().unwrap();
        assert_eq!(edits.len(), 1);
        assert!(
            edits[0].new_text.contains("Checkout page"),
            "got: {:?}",
            edits[0].new_text
        );
    }

    #[test]
    fn req_rnm_03_escapes_special_chars_in_new_name() {
        let content = "msgid \"A\"\nmsgstr \"\"\n";
        let e = entry("A", "/locale/de/messages.po", 1);
        let index = CatalogIndex::build(vec![e]);
        let key = CatalogKey::new("A");
        let mut bufs = HashMap::new();
        bufs.insert(PathBuf::from("/locale/de/messages.po"), content.to_string());
        let edit = build_rename_edit(&key, "Say \"hi\"", &index, &bufs, &[]).unwrap();
        let changes = edit.changes.unwrap();
        let new_text = &changes.values().next().unwrap()[0].new_text;
        assert!(new_text.contains("\\\"hi\\\""), "got: {new_text}");
    }

    // ── REQ-RNM-04: source call sites are rewritten ──────────────────────────

    #[test]
    fn req_rnm_04_rewrites_source_call_site() {
        let e = entry("Checkout", "/locale/de/messages.po", 1);
        let index = CatalogIndex::build(vec![e]);
        let key = CatalogKey::new("Checkout");
        let mut bufs = HashMap::new();
        bufs.insert(
            PathBuf::from("/locale/de/messages.po"),
            "msgid \"Checkout\"\nmsgstr \"Kasse\"\n".to_string(),
        );
        let source_calls = calls(r#"_("Checkout")"#);
        let call_sites = vec![(uri("/app/views.py"), source_calls)];
        let edit = build_rename_edit(&key, "Checkout page", &index, &bufs, &call_sites).unwrap();
        let changes = edit.changes.unwrap();
        let views_uri = uri("/app/views.py");
        let source_edits = &changes[&views_uri];
        assert_eq!(source_edits.len(), 1);
        assert_eq!(source_edits[0].new_text, "Checkout page");
    }

    // ── REQ-RNM-07: collision aborts with a message ──────────────────────────

    #[test]
    fn req_rnm_07_collision_returns_error() {
        let checkout = entry("Checkout", "/locale/de/messages.po", 1);
        let cart = entry("Cart", "/locale/de/messages.po", 5);
        let index = CatalogIndex::build(vec![checkout, cart]);
        let key = CatalogKey::new("Checkout");
        let bufs = HashMap::new();
        let result = build_rename_edit(&key, "Cart", &index, &bufs, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cart"));
    }

    #[test]
    fn req_rnm_07_source_side_collision_returns_error() {
        // Renaming "Checkout" to "Cart" where "Cart" already exists as a call site msgid.
        let e = entry("Checkout", "/locale/de/messages.po", 1);
        let index = CatalogIndex::build(vec![e]);
        let key = CatalogKey::new("Checkout");
        let bufs = HashMap::new();
        // A source file that has both _("Checkout") and _("Cart").
        let source_calls = calls(r#"_("Checkout"); _("Cart")"#);
        let call_sites = vec![(uri("/app/views.py"), source_calls)];
        let result = build_rename_edit(&key, "Cart", &index, &bufs, &call_sites);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cart"));
    }

    #[test]
    fn req_rnm_07_no_collision_when_target_is_free() {
        let e = entry("Checkout", "/locale/de/messages.po", 1);
        let index = CatalogIndex::build(vec![e]);
        let key = CatalogKey::new("Checkout");
        let mut bufs = HashMap::new();
        bufs.insert(
            PathBuf::from("/locale/de/messages.po"),
            "msgid \"Checkout\"\nmsgstr \"Kasse\"\n".to_string(),
        );
        let result = build_rename_edit(&key, "Checkout page", &index, &bufs, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn req_rnm_02_prepare_catalog_rejects_multiline_first_line() {
        // Multi-line msgid: first line is `msgid ""`, continuation on next line.
        // Cursor on the `msgid ""` line — the quoted literal there is empty (zero-width
        // range); prepare should return None rather than an empty selection.
        let content = "msgid \"\"\n\"Checkout\"\nmsgstr \"Kasse\"\n";
        let lines: Vec<&str> = content.lines().collect();
        let e = entry("Checkout", "/locale/de/messages.po", 1); // entry.msgid = "Checkout"
        let result = prepare_rename_catalog(&[&e], &lines, pos(0, 0));
        assert!(result.is_none());
    }

    // ── REQ-RNM-03: multi-locale edit ────────────────────────────────────────

    #[test]
    fn req_rnm_03_builds_workspace_edit_across_pot_and_multiple_locales() {
        let pot = {
            let mut e = entry("Checkout", "/locale/messages.pot", 1);
            e.locale = "".into();
            e
        };
        let de = entry("Checkout", "/locale/de/messages.po", 1);
        let fr = {
            let mut e = entry("Checkout", "/locale/fr/messages.po", 1);
            e.locale = "fr".into();
            e.msgstr = vec!["".into()];
            e
        };
        let index = CatalogIndex::build(vec![pot, de, fr]);
        let key = CatalogKey::new("Checkout");
        let po_content = "msgid \"Checkout\"\nmsgstr \"\"\n".to_string();
        let mut bufs = HashMap::new();
        bufs.insert(PathBuf::from("/locale/messages.pot"), po_content.clone());
        bufs.insert(
            PathBuf::from("/locale/de/messages.po"),
            "msgid \"Checkout\"\nmsgstr \"Kasse\"\n".to_string(),
        );
        bufs.insert(PathBuf::from("/locale/fr/messages.po"), po_content);
        let edit = build_rename_edit(&key, "Checkout page", &index, &bufs, &[]).unwrap();
        let changes = edit.changes.unwrap();
        assert_eq!(changes.len(), 3, "should touch pot + de + fr");
        for edits in changes.values() {
            assert_eq!(edits.len(), 1);
            assert!(
                edits[0].new_text.contains("Checkout page"),
                "got: {:?}",
                edits[0].new_text
            );
        }
    }

    // ── Multi-line msgid ─────────────────────────────────────────────────────

    #[test]
    fn multiline_msgid_block_replaced_correctly() {
        let content = "msgid \"\"\n\"Checkout\"\nmsgstr \"Kasse\"\n";
        // polib would give entry.msgid = "Checkout" (concatenated)
        let e = entry("Checkout", "/locale/de/messages.po", 1);
        let index = CatalogIndex::build(vec![e]);
        let key = CatalogKey::new("Checkout");
        let mut bufs = HashMap::new();
        bufs.insert(PathBuf::from("/locale/de/messages.po"), content.to_string());
        let edit = build_rename_edit(&key, "Checkout page", &index, &bufs, &[]).unwrap();
        let changes = edit.changes.unwrap();
        let text_edit = &changes.values().next().unwrap()[0];
        // The range should start at the msgid keyword line (line 0) and extend
        // to include the continuation (end at msgstr start = line 2).
        assert_eq!(text_edit.range.start.line, 0);
        assert_eq!(text_edit.range.end.line, 2); // msgstr at line 2
        assert!(text_edit.new_text.contains("Checkout page"));
        // The continuation line is consumed by the range, so no more "Checkout" fragment.
        assert!(!text_edit.new_text.contains("\"Checkout\""));
    }
}
