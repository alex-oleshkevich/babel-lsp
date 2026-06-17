use tower_lsp_server::ls_types::*;

use crate::catalog::index::{CatalogIndex, CatalogKey};
use crate::extract::types::TranslationCall;
use crate::util::pos_in_range;

/// REQ-NAV-01/02: return locations for all catalog entries that define the key,
/// with the `.pot` template entry first, then each locale sorted alphabetically.
pub fn goto_definition(
    calls: &[TranslationCall],
    pos: Position,
    index: &CatalogIndex,
) -> Option<GotoDefinitionResponse> {
    let call = calls
        .iter()
        .find(|c| c.msgid_range.is_some_and(|r| pos_in_range(pos, r)))?;
    let msgid = call.msgid.as_deref()?;
    let key = CatalogKey {
        msgid: msgid.to_string(),
        msgctxt: call.msgctxt.clone(),
    };

    let mut locations: Vec<Location> = Vec::new();

    // .pot entry first (REQ-NAV-02).
    if let Some(pot) = index.lookup_pot(&key) {
        if let Some(uri) = Uri::from_file_path(&pot.file_path) {
            locations.push(entry_location(uri, pot.line));
        }
    }

    // .po entries sorted by locale.
    let mut po_entries: Vec<_> = index.lookup(&key).iter().collect();
    po_entries.sort_by(|a, b| a.locale.cmp(&b.locale));
    for entry in po_entries {
        if let Some(uri) = Uri::from_file_path(&entry.file_path) {
            locations.push(entry_location(uri, entry.line));
        }
    }

    if locations.is_empty() {
        return None;
    }

    Some(GotoDefinitionResponse::Array(locations))
}

fn entry_location(uri: Uri, line_1based: u32) -> Location {
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

    fn pot_entry(msgid: &str, path: &str) -> CatalogEntry {
        let mut e = entry("", msgid, path);
        e.line = 3;
        e
    }

    fn goto_at(
        src: &str,
        line: u32,
        ch: u32,
        index: &CatalogIndex,
    ) -> Option<GotoDefinitionResponse> {
        let calls = python::extract(src.as_bytes(), &no_extra());
        goto_definition(
            &calls,
            Position {
                line,
                character: ch,
            },
            index,
        )
    }

    // ── REQ-NAV-01 ───────────────────────────────────────────────────────────

    #[test]
    fn req_nav_01_goto_returns_a_location_per_catalog_entry() {
        let index = CatalogIndex::build(vec![
            entry("de", "Checkout", "/locale/de/messages.po"),
            entry("fr", "Checkout", "/locale/fr/messages.po"),
        ]);
        let resp = goto_at(r#"_("Checkout")"#, 0, 5, &index).unwrap();
        let locs = match resp {
            GotoDefinitionResponse::Array(v) => v,
            _ => panic!("expected Array"),
        };
        assert_eq!(locs.len(), 2);
        assert!(locs[0].uri.to_string().contains("de"));
        assert!(locs[1].uri.to_string().contains("fr"));
    }

    #[test]
    fn req_nav_01_cursor_outside_msgid_returns_none() {
        let index = CatalogIndex::build(vec![entry("de", "Checkout", "/locale/de/messages.po")]);
        let resp = goto_at(r#"_("Checkout")"#, 0, 0, &index);
        assert!(resp.is_none());
    }

    #[test]
    fn req_nav_01_fstring_returns_none() {
        let index = CatalogIndex::build(vec![entry("de", "Checkout", "/locale/de/messages.po")]);
        let calls = python::extract(br#"_(f"Checkout")"#, &no_extra());
        let resp = goto_definition(
            &calls,
            Position {
                line: 0,
                character: 5,
            },
            &index,
        );
        assert!(resp.is_none());
    }

    // ── REQ-NAV-02 ───────────────────────────────────────────────────────────

    #[test]
    fn req_nav_02_goto_orders_pot_first_then_locales() {
        let index = CatalogIndex::build(vec![
            pot_entry("Checkout", "/locale/messages.pot"),
            entry("fr", "Checkout", "/locale/fr/messages.po"),
            entry("de", "Checkout", "/locale/de/messages.po"),
        ]);
        let resp = goto_at(r#"_("Checkout")"#, 0, 5, &index).unwrap();
        let locs = match resp {
            GotoDefinitionResponse::Array(v) => v,
            _ => panic!("expected Array"),
        };
        assert_eq!(locs.len(), 3);
        // .pot first
        assert!(
            locs[0].uri.to_string().contains("messages.pot"),
            "pot not first: {:?}",
            locs[0].uri
        );
        // then de < fr alphabetically
        assert!(locs[1].uri.to_string().contains("de"));
        assert!(locs[2].uri.to_string().contains("fr"));
    }

    #[test]
    fn req_nav_02_line_converts_to_zero_based() {
        let mut e = entry("de", "Checkout", "/locale/de/messages.po");
        e.line = 5; // 1-based
        let index = CatalogIndex::build(vec![e]);
        let resp = goto_at(r#"_("Checkout")"#, 0, 5, &index).unwrap();
        let locs = match resp {
            GotoDefinitionResponse::Array(v) => v,
            _ => panic!(),
        };
        assert_eq!(locs[0].range.start.line, 4); // 0-based
    }
}
