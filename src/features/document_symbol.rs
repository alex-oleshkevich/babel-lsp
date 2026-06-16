use tower_lsp_server::ls_types::{
    DocumentSymbol, Location, OneOf, Position, Range, SymbolKind, Uri, WorkspaceSymbol,
};

use crate::catalog::index::{CatalogEntry, CatalogIndex, CatalogKey};

const DETAIL_TRUNCATE: usize = 60;

/// Document symbols: one `STRING` symbol per catalog entry in file order (REQ-SYM-01).
pub fn document_symbols(entries: &[&CatalogEntry]) -> Vec<DocumentSymbol> {
    entries
        .iter()
        .filter(|e| !e.msgid.is_empty())
        .map(|e| {
            let pos = entry_position(e);
            let rng = Range { start: pos, end: pos };
            #[allow(deprecated)]
            DocumentSymbol {
                name: symbol_name(&e.msgid, e.msgctxt.as_deref()),
                detail: Some(entry_detail(e)),
                kind: SymbolKind::STRING,
                tags: None,
                deprecated: None,
                range: rng,
                selection_range: rng,
                children: None,
            }
        })
        .collect()
}

/// Workspace symbols: case-insensitive substring match over every msgid (REQ-SYM-04).
pub fn workspace_symbols(index: &CatalogIndex, query: &str) -> Vec<WorkspaceSymbol> {
    index
        .all_msgids()
        .filter(|key| query_matches(key, query))
        .filter_map(|key| {
            // REQ-SYM-05: .pot entry first; else first .po entry; one result per key.
            let entry = index
                .lookup_pot(key)
                .or_else(|| index.lookup(key).first())?;
            let uri = Uri::from_file_path(&entry.file_path)?;
            let pos = entry_position(entry);
            let rng = Range { start: pos, end: pos };
            Some(WorkspaceSymbol {
                name: symbol_name(&key.msgid, key.msgctxt.as_deref()),
                kind: SymbolKind::STRING,
                tags: None,
                container_name: None,
                location: OneOf::Left(Location { uri, range: rng }),
                data: None,
            })
        })
        .collect()
}

/// `msgctxt|msgid` when context is present, plain msgid otherwise (REQ-SYM-06).
fn symbol_name(msgid: &str, msgctxt: Option<&str>) -> String {
    match msgctxt {
        Some(ctx) => format!("{ctx}|{msgid}"),
        None => msgid.to_string(),
    }
}

/// `fuzzy` | `untranslated` | truncated msgstr (REQ-SYM-02).
fn entry_detail(entry: &CatalogEntry) -> String {
    if entry.flags.fuzzy {
        return "fuzzy".to_string();
    }
    if entry.msgstr.iter().all(|s| s.is_empty()) {
        return "untranslated".to_string();
    }
    truncate(&entry.msgstr[0], DETAIL_TRUNCATE)
}

fn truncate(s: &str, limit: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() { format!("{head}…") } else { head }
}

/// Convert a 1-based catalog line to a 0-based LSP position.
fn entry_position(entry: &CatalogEntry) -> Position {
    Position { line: entry.line.saturating_sub(1), character: 0 }
}

/// Match the query against msgid and msgctxt so "button" finds "button|Save" (REQ-SYM-06).
fn query_matches(key: &CatalogKey, query: &str) -> bool {
    if query.is_empty() { return true; }
    let q = query.to_lowercase();
    key.msgid.to_lowercase().contains(&q)
        || key.msgctxt.as_deref().is_some_and(|ctx| ctx.to_lowercase().contains(&q))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::catalog::index::{CatalogEntry, EntryFlags};

    fn entry(locale: &str, msgid: &str, msgstr: &str) -> CatalogEntry {
        CatalogEntry {
            locale: locale.into(),
            domain: "messages".into(),
            msgid: msgid.into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec![msgstr.into()],
            flags: EntryFlags { fuzzy: false, obsolete: false },
            file_path: PathBuf::from("/locale/de/messages.po"),
            line: 5,
        }
    }

    fn entry_ctx(locale: &str, msgid: &str, msgctxt: &str, msgstr: &str) -> CatalogEntry {
        let mut e = entry(locale, msgid, msgstr);
        e.msgctxt = Some(msgctxt.into());
        e
    }

    fn fuzzy_entry(locale: &str, msgid: &str, msgstr: &str) -> CatalogEntry {
        let mut e = entry(locale, msgid, msgstr);
        e.flags.fuzzy = true;
        e
    }

    // ── document_symbols ────────────────────────────────────────────────────

    // REQ-SYM-01 — one symbol per entry, header skipped

    #[test]
    fn req_sym_01_one_symbol_per_entry() {
        let e1 = entry("de", "Checkout", "Kasse");
        let e2 = entry("de", "Save", "Speichern");
        let refs: Vec<&CatalogEntry> = vec![&e1, &e2];
        let syms = document_symbols(&refs);
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "Checkout");
        assert_eq!(syms[1].name, "Save");
    }

    #[test]
    fn req_sym_01_header_entry_skipped() {
        let header = entry("de", "", "");
        let real = entry("de", "Checkout", "Kasse");
        let refs: Vec<&CatalogEntry> = vec![&header, &real];
        let syms = document_symbols(&refs);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Checkout");
    }

    #[test]
    fn req_sym_01_position_is_zero_based() {
        let mut e = entry("de", "Checkout", "Kasse");
        e.line = 10; // 1-based → should become line 9
        let refs: Vec<&CatalogEntry> = vec![&e];
        let syms = document_symbols(&refs);
        assert_eq!(syms[0].range.start.line, 9);
    }

    #[test]
    fn req_sym_01_kind_is_string() {
        let e = entry("de", "Checkout", "Kasse");
        let syms = document_symbols(&[&e]);
        assert_eq!(syms[0].kind, SymbolKind::STRING);
    }

    // REQ-SYM-02 — detail

    #[test]
    fn req_sym_02_translated_detail_is_msgstr() {
        let e = entry("de", "Checkout", "Kasse");
        let syms = document_symbols(&[&e]);
        assert_eq!(syms[0].detail.as_deref(), Some("Kasse"));
    }

    #[test]
    fn req_sym_02_fuzzy_detail_is_fuzzy() {
        let e = fuzzy_entry("de", "Save", "Speichern");
        let syms = document_symbols(&[&e]);
        assert_eq!(syms[0].detail.as_deref(), Some("fuzzy"));
    }

    #[test]
    fn req_sym_02_empty_msgstr_detail_is_untranslated() {
        let e = entry("de", "Checkout", "");
        let syms = document_symbols(&[&e]);
        assert_eq!(syms[0].detail.as_deref(), Some("untranslated"));
    }

    #[test]
    fn req_sym_02_long_msgstr_is_truncated() {
        let long = "A".repeat(80);
        let e = entry("de", "Key", &long);
        let syms = document_symbols(&[&e]);
        let detail = syms[0].detail.as_deref().unwrap_or("");
        assert!(detail.contains('…'), "expected truncation: {detail:?}");
    }

    // REQ-SYM-03 — non-catalog returns nothing
    // (enforced by the server handler, not this function — tested at server level)

    // REQ-SYM-06 — context in name

    #[test]
    fn req_sym_06_context_in_name() {
        let e = entry_ctx("de", "Save", "button", "Speichern");
        let syms = document_symbols(&[&e]);
        assert_eq!(syms[0].name, "button|Save");
    }

    #[test]
    fn req_sym_06_no_context_is_plain_name() {
        let e = entry("de", "Save", "Speichern");
        let syms = document_symbols(&[&e]);
        assert_eq!(syms[0].name, "Save");
    }

    // ── workspace_symbols ────────────────────────────────────────────────────

    fn make_index_with(entries: Vec<CatalogEntry>) -> CatalogIndex {
        CatalogIndex::build(entries)
    }

    // REQ-SYM-04 — query matching

    #[test]
    fn req_sym_04_empty_query_returns_all() {
        let index = make_index_with(vec![
            entry("de", "Checkout", "Kasse"),
            entry("de", "Save", "Speichern"),
        ]);
        let syms = workspace_symbols(&index, "");
        assert_eq!(syms.len(), 2);
    }

    #[test]
    fn req_sym_04_substring_match_case_insensitive() {
        let index = make_index_with(vec![
            entry("de", "Checkout", "Kasse"),
            entry("de", "Save", "Speichern"),
        ]);
        let syms = workspace_symbols(&index, "chec");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Checkout");
    }

    #[test]
    fn req_sym_04_uppercase_query_matches_lowercase_msgid() {
        let index = make_index_with(vec![entry("de", "checkout", "Kasse")]);
        let syms = workspace_symbols(&index, "CHEC");
        assert_eq!(syms.len(), 1);
    }

    #[test]
    fn req_sym_04_no_match_returns_empty() {
        let index = make_index_with(vec![entry("de", "Checkout", "Kasse")]);
        let syms = workspace_symbols(&index, "xyz");
        assert!(syms.is_empty());
    }

    // REQ-SYM-05 — one result per key, .pot first

    #[test]
    fn req_sym_05_prefers_pot_entry() {
        let mut pot = entry("", "Checkout", "");
        pot.file_path = PathBuf::from("/locale/messages.pot");
        let po = entry("de", "Checkout", "Kasse");
        let index = make_index_with(vec![pot, po]);
        let syms = workspace_symbols(&index, "Checkout");
        assert_eq!(syms.len(), 1);
        let location = match &syms[0].location {
            OneOf::Left(loc) => loc,
            _ => panic!("expected location"),
        };
        assert!(location.uri.to_string().ends_with(".pot"), "should prefer .pot: {:?}", location.uri);
    }

    #[test]
    fn req_sym_05_falls_back_to_po_when_no_pot() {
        let index = make_index_with(vec![
            entry("de", "Checkout", "Kasse"),
            entry("fr", "Checkout", "Caisse"),
        ]);
        let syms = workspace_symbols(&index, "Checkout");
        // Only one result per key (not per locale)
        assert_eq!(syms.len(), 1);
    }

    // REQ-SYM-06 — context in workspace symbol name

    #[test]
    fn req_sym_06_workspace_context_in_name() {
        let e = entry_ctx("de", "Save", "button", "Speichern");
        let index = make_index_with(vec![e]);
        let syms = workspace_symbols(&index, "button");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "button|Save");
    }
}
