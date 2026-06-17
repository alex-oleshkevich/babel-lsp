use std::collections::HashSet;

use tower_lsp_server::ls_types::*;

use crate::catalog::index::{CatalogIndex, CatalogKey};
use crate::extract::types::TranslationCall;

/// REQ-NAV-04/05: collect all locations that reference `key`.
///
/// `call_sites` supplies `(uri, calls)` pairs from the caller — open editor
/// documents and workspace-scanned files combined.  The index contributes the
/// catalog entries.  Each `(uri, range)` pair is emitted at most once
/// (REQ-NAV-05).
pub fn find_references(
    key: &CatalogKey,
    index: &CatalogIndex,
    call_sites: impl IntoIterator<Item = (Uri, Vec<TranslationCall>)>,
) -> Vec<Location> {
    let mut seen: HashSet<(String, u32, u32)> = HashSet::new();
    let mut locations: Vec<Location> = Vec::new();

    let mut add = |loc: Location| {
        let dedup_key = (
            loc.uri.to_string(),
            loc.range.start.line,
            loc.range.start.character,
        );
        if seen.insert(dedup_key) {
            locations.push(loc);
        }
    };

    // Catalog entries (pot + po).
    if let Some(pot) = index.lookup_pot(key) {
        if let Some(uri) = Uri::from_file_path(&pot.file_path) {
            add(catalog_location(uri, pot.line));
        }
    }
    let mut po_entries: Vec<_> = index.lookup(key).iter().collect();
    po_entries.sort_by(|a, b| a.locale.cmp(&b.locale));
    for entry in po_entries {
        if let Some(uri) = Uri::from_file_path(&entry.file_path) {
            add(catalog_location(uri, entry.line));
        }
    }

    // Source call sites (open docs + workspace scan).
    for (uri, calls) in call_sites {
        for call in &calls {
            if call_matches_key(call, key) {
                if let Some(range) = call.msgid_range {
                    add(Location {
                        uri: uri.clone(),
                        range,
                    });
                }
            }
        }
    }

    locations
}

fn call_matches_key(call: &TranslationCall, key: &CatalogKey) -> bool {
    call.msgid.as_deref() == Some(key.msgid.as_str())
        && call.msgctxt.as_deref() == key.msgctxt.as_deref()
}

fn catalog_location(uri: Uri, line_1based: u32) -> Location {
    let line = line_1based.saturating_sub(1);
    Location {
        uri,
        range: Range {
            start: Position { line, character: 0 },
            end: Position { line, character: 0 },
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::catalog::index::{CatalogEntry, CatalogIndex, EntryFlags};
    use crate::extract::python;

    fn no_extra() -> std::collections::HashMap<String, crate::extract::types::TranslationFunc> {
        std::collections::HashMap::new()
    }

    fn entry(locale: &str, msgid: &str, path: &str) -> CatalogEntry {
        CatalogEntry {
            locale: locale.into(),
            domain: "messages".into(),
            msgid: msgid.into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec!["".into()],
            flags: EntryFlags {
                fuzzy: false,
                obsolete: false,
            },
            file_path: PathBuf::from(path),
            line: 5,
        }
    }

    fn make_uri(path: &str) -> Uri {
        Uri::from_file_path(path).unwrap()
    }

    fn calls_for(src: &str) -> Vec<TranslationCall> {
        python::extract(src.as_bytes(), &no_extra())
    }

    // ── REQ-NAV-04 ───────────────────────────────────────────────────────────

    #[test]
    fn req_nav_04_references_aggregate_entries_and_calls() {
        let index = CatalogIndex::build(vec![
            entry("de", "Checkout", "/locale/de/messages.po"),
            entry("fr", "Checkout", "/locale/fr/messages.po"),
        ]);
        let key = CatalogKey::new("Checkout");
        let src_calls = calls_for(r#"_("Checkout")"#);
        let call_sites = vec![(make_uri("/app/views.py"), src_calls)];
        let locs = find_references(&key, &index, call_sites);

        // 2 catalog + 1 source = 3 locations
        assert_eq!(locs.len(), 3);
        let uris: Vec<String> = locs.iter().map(|l| l.uri.to_string()).collect();
        assert!(uris.iter().any(|u| u.contains("de")));
        assert!(uris.iter().any(|u| u.contains("fr")));
        assert!(uris.iter().any(|u| u.contains("views")));
    }

    #[test]
    fn req_nav_04_different_key_not_included() {
        let index = CatalogIndex::build(vec![entry("de", "Save", "/locale/de/messages.po")]);
        let key = CatalogKey::new("Checkout");
        let locs = find_references(&key, &index, std::iter::empty());
        assert!(locs.is_empty(), "Save entry should not match Checkout key");
    }

    // ── REQ-NAV-05 ───────────────────────────────────────────────────────────

    #[test]
    fn req_nav_05_references_dedup_by_uri_range() {
        let index = CatalogIndex::build(vec![entry("de", "Checkout", "/locale/de/messages.po")]);
        let key = CatalogKey::new("Checkout");
        let src_calls = calls_for(r#"_("Checkout")"#);
        let uri = make_uri("/app/views.py");
        // Same calls provided twice (simulating open-doc + disk-scan overlap).
        let call_sites = vec![
            (uri.clone(), src_calls.clone()),
            (uri.clone(), src_calls.clone()),
        ];
        let locs = find_references(&key, &index, call_sites);
        // 1 catalog + 1 source (deduplicated from 2 inputs) = 2
        assert_eq!(locs.len(), 2);
    }

    // ── REQ-NAV-06 is tested via the server handler (workspace scan) ──────────

    // ── msgctxt matching ──────────────────────────────────────────────────────

    #[test]
    fn references_respects_msgctxt() {
        let index = CatalogIndex::build(vec![entry("de", "Save", "/locale/de/messages.po")]);
        let key = CatalogKey::with_ctx("Save", "button");
        let calls = calls_for(r#"pgettext("button", "Save")"#);
        let call_sites = vec![(make_uri("/app/views.py"), calls)];
        let locs = find_references(&key, &index, call_sites);
        // Index has plain "Save" (no context), call has context "button" → no catalog match.
        // But the source call matches (pgettext with "button"/"Save").
        let uris: Vec<String> = locs.iter().map(|l| l.uri.to_string()).collect();
        assert!(uris.iter().any(|u| u.contains("views")));
        assert!(
            !uris.iter().any(|u| u.contains("de")),
            "context mismatch should exclude catalog entry"
        );
    }
}
