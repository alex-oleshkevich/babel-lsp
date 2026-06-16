use tower_lsp_server::ls_types::*;

use crate::catalog::index::{CatalogEntry, CatalogIndex, CatalogKey};
use crate::extract::types::TranslationCall;

/// Hover card anchored to the msgid literal for a source file (Python/Jinja).
///
/// Finds the call whose `msgid_range` contains `pos`. Returns `None` when the
/// cursor is outside every call's string literal or the call has no literal
/// msgid (f-string, variable, etc.).
pub fn hover_source(
    calls: &[TranslationCall],
    pos: Position,
    index: &CatalogIndex,
) -> Option<Hover> {
    let call = calls
        .iter()
        .find(|c| c.msgid_range.is_some_and(|r| pos_in_range(pos, r)))?;
    let msgid = call.msgid.as_deref()?;
    let key = CatalogKey {
        msgid: msgid.to_string(),
        msgctxt: call.msgctxt.clone(),
    };
    let entries = index.lookup(&key);
    render_card(&key, call.msgid_plural.as_deref(), call.msgid_range, entries)
}

/// Hover card for a catalog (.po/.pot) buffer.
///
/// Finds the entry whose 1-based `line` equals `pos.line + 1`, then looks up
/// all locales for that key in the full index so the card shows every locale,
/// not just the one being edited.
pub fn hover_catalog(
    file_entries: &[&CatalogEntry],
    pos: Position,
    index: &CatalogIndex,
) -> Option<Hover> {
    let cursor_line = pos.line + 1; // 1-based
    let hit = file_entries.iter().find(|e| e.line == cursor_line)?;
    let key = hit.key();
    let entries = index.lookup(&key);
    render_card(&key, hit.msgid_plural.as_deref(), None, entries)
}

/// Build the markdown hover card.
fn render_card(
    key: &CatalogKey,
    msgid_plural: Option<&str>,
    range: Option<Range>,
    entries: &[CatalogEntry],
) -> Option<Hover> {
    let mut md = String::new();

    // Header — id, optional context, optional plural.
    md.push_str(&format!("**msgid** `{}`\n", key.msgid));
    if let Some(ctx) = &key.msgctxt {
        md.push('\n');
        md.push_str(&format!("**context** `{ctx}`\n"));
    }
    if let Some(plural) = msgid_plural {
        md.push('\n');
        md.push_str(&format!("**plural** `{plural}`\n"));
    }
    md.push('\n');

    if entries.is_empty() {
        // REQ-HOV-04: typo case — no catalog knows this msgid.
        md.push_str("No translations found.");
    } else {
        // REQ-HOV-02/03: per-locale table sorted by locale then domain.
        md.push_str("| Locale | Domain | Translation | Status |\n");
        md.push_str("|--------|--------|-------------|--------|\n");

        let mut sorted: Vec<&CatalogEntry> = entries.iter().collect();
        sorted.sort_by(|a, b| a.locale.cmp(&b.locale).then(a.domain.cmp(&b.domain)));

        for entry in sorted {
            let (translation, status) = entry_translation_and_status(entry);
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                entry.locale, entry.domain, translation, status
            ));
        }
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: md,
        }),
        range,
    })
}

/// REQ-HOV-03: returns (translation_cell, status_label) for one entry.
fn entry_translation_and_status(entry: &CatalogEntry) -> (String, &'static str) {
    if entry.flags.fuzzy {
        let t = entry
            .msgstr
            .iter()
            .find(|s| !s.is_empty())
            .map(String::as_str)
            .unwrap_or("—")
            .to_string();
        (t, "fuzzy")
    } else if let Some(t) = entry.msgstr.iter().find(|s| !s.is_empty()) {
        (t.clone(), "ok")
    } else {
        ("—".to_string(), "missing")
    }
}

/// Returns true when `pos` is within `range` (inclusive start, exclusive end).
fn pos_in_range(pos: Position, range: Range) -> bool {
    if pos.line < range.start.line || pos.line > range.end.line {
        return false;
    }
    if pos.line == range.start.line && pos.character < range.start.character {
        return false;
    }
    if pos.line == range.end.line && pos.character >= range.end.character {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::catalog::index::{CatalogIndex, EntryFlags};
    use crate::extract::python;

    fn no_extra() -> std::collections::HashMap<String, crate::extract::types::TranslationFunc> {
        std::collections::HashMap::new()
    }

    fn entry(locale: &str, msgid: &str, msgstr: &str) -> CatalogEntry {
        CatalogEntry {
            locale: locale.into(),
            domain: "messages".into(),
            msgid: msgid.into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec![msgstr.into()],
            flags: EntryFlags { fuzzy: false, obsolete: false },
            file_path: PathBuf::from("/locale/messages.po"),
            line: 5,
        }
    }

    fn entry_fuzzy(locale: &str, msgid: &str, msgstr: &str) -> CatalogEntry {
        let mut e = entry(locale, msgid, msgstr);
        e.flags.fuzzy = true;
        e
    }

    fn entry_ctx(locale: &str, msgid: &str, msgctxt: &str, msgstr: &str) -> CatalogEntry {
        let mut e = entry(locale, msgid, msgstr);
        e.msgctxt = Some(msgctxt.into());
        e
    }

    fn shopfront_index() -> CatalogIndex {
        CatalogIndex::build(vec![
            entry("de", "Checkout", "Kasse"),
            entry("fr", "Checkout", ""),
            entry_fuzzy("de", "Save", "Speichern"),
        ])
    }

    fn hover_at(src: &str, line: u32, character: u32, index: &CatalogIndex) -> Option<Hover> {
        let calls = python::extract(src.as_bytes(), &no_extra());
        hover_source(&calls, Position { line, character }, index)
    }

    fn card_text(h: &Hover) -> &str {
        match &h.contents {
            HoverContents::Markup(mc) => &mc.value,
            _ => panic!("expected Markup"),
        }
    }

    // ── REQ-HOV-01 ───────────────────────────────────────────────────────────

    #[test]
    fn req_hov_01_dispatches_on_source_call() {
        let index = shopfront_index();
        let h = hover_at(r#"_("Checkout")"#, 0, 5, &index).expect("hover expected");
        assert!(card_text(&h).contains("Checkout"));
    }

    #[test]
    fn req_hov_01_cursor_outside_msgid_returns_none() {
        let index = shopfront_index();
        // Cursor on function name `_`, outside the string literal → None.
        let h = hover_at(r#"_("Checkout")"#, 0, 0, &index);
        assert!(h.is_none());
    }

    #[test]
    fn req_hov_01_dispatches_on_catalog_entry() {
        let index = shopfront_index();
        // Simulate a .po buffer: Checkout entry at line 5.
        let checkout = entry("de", "Checkout", "Kasse");
        let file_entries: Vec<&CatalogEntry> = vec![&checkout];
        // CatalogEntry.line is 1-based (5); cursor pos.line is 0-based (4).
        let h = hover_catalog(&file_entries, Position { line: 4, character: 0 }, &index)
            .expect("hover expected");
        assert!(card_text(&h).contains("Checkout"));
    }

    #[test]
    fn req_hov_01_catalog_cursor_on_wrong_line_returns_none() {
        let index = shopfront_index();
        let checkout = entry("de", "Checkout", "Kasse");
        let file_entries: Vec<&CatalogEntry> = vec![&checkout];
        let h = hover_catalog(&file_entries, Position { line: 0, character: 0 }, &index);
        assert!(h.is_none());
    }

    // ── REQ-HOV-02 ───────────────────────────────────────────────────────────

    #[test]
    fn req_hov_02_renders_id_context_plural_header() {
        let index = CatalogIndex::build(vec![entry_ctx("de", "Save", "button", "Speichern")]);
        let h = hover_at(r#"pgettext("button", "Save")"#, 0, 20, &index).unwrap();
        let text = card_text(&h);
        assert!(text.contains("**msgid** `Save`"), "id missing");
        assert!(text.contains("**context** `button`"), "context missing");
        assert!(!text.contains("**plural**"), "plural should be absent");
    }

    #[test]
    fn req_hov_02_renders_per_locale_table() {
        let index = shopfront_index();
        let h = hover_at(r#"_("Checkout")"#, 0, 5, &index).unwrap();
        let text = card_text(&h);
        assert!(text.contains("| Locale | Domain | Translation | Status |"), "table header missing");
        assert!(text.contains("de"), "de row missing");
        assert!(text.contains("fr"), "fr row missing");
    }

    #[test]
    fn req_hov_02_plural_line_present_for_ngettext() {
        let index = CatalogIndex::build(vec![entry("de", "%(n)d item", "%(n)d Eintrag")]);
        let h = hover_at(r#"ngettext("%(n)d item", "%(n)d items", n)"#, 0, 10, &index).unwrap();
        let text = card_text(&h);
        assert!(text.contains("**plural** `%(n)d items`"), "plural line missing");
    }

    // ── REQ-HOV-03 ───────────────────────────────────────────────────────────

    #[test]
    fn req_hov_03_status_is_ok_fuzzy_or_missing() {
        let index = shopfront_index();
        // Save: de is fuzzy, no fr entry.
        let h = hover_at(r#"_("Save")"#, 0, 4, &index).unwrap();
        let text = card_text(&h);
        assert!(text.contains("fuzzy"), "fuzzy status missing");

        // Checkout: de is ok, fr is missing (empty msgstr).
        let h = hover_at(r#"_("Checkout")"#, 0, 5, &index).unwrap();
        let text = card_text(&h);
        assert!(text.contains("ok"), "ok status missing");
        assert!(text.contains("missing"), "missing status missing");
        assert!(text.contains("—"), "em-dash for missing translation missing");
    }

    // ── REQ-HOV-04 ───────────────────────────────────────────────────────────

    #[test]
    fn req_hov_04_no_entries_says_no_translations_found() {
        let index = shopfront_index();
        let h = hover_at(r#"_("Chekout")"#, 0, 5, &index).unwrap();
        let text = card_text(&h);
        assert!(text.contains("Chekout"), "msgid missing from card");
        assert!(text.contains("No translations found."), "message missing");
        assert!(!text.contains("| Locale |"), "table should be absent");
    }

    // ── REQ-HOV-05 ───────────────────────────────────────────────────────────

    #[test]
    fn req_hov_05_anchors_to_msgid_range() {
        let index = shopfront_index();
        // _("Checkout") — "Checkout" string node spans cols 2..12
        let h = hover_at(r#"_("Checkout")"#, 0, 5, &index).unwrap();
        let range = h.range.expect("hover range must be set");
        assert_eq!(range.start, Position { line: 0, character: 2 }); // opening quote
        assert_eq!(range.end, Position { line: 0, character: 12 }); // after closing quote
    }

    #[test]
    fn req_hov_05_fstring_msgid_returns_none() {
        let index = shopfront_index();
        // f-string → call.msgid is None → hover returns None.
        let calls = python::extract(br#"_(f"Hello {user}")"#, &no_extra());
        let h = hover_source(&calls, Position { line: 0, character: 5 }, &index);
        assert!(h.is_none());
    }
}
