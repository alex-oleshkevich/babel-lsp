use tower_lsp_server::ls_types::{InlayHint, InlayHintKind, InlayHintLabel, Range};

use crate::catalog::index::{CatalogIndex, CatalogKey};
use crate::extract::types::TranslationCall;

const TRUNCATE_CHARS: usize = 40;

/// Produce inlay hints for translation calls within `range`.
///
/// Returns an empty list when the locale is `None` (REQ-HINT-01).
pub fn inlay_hints(
    calls: &[TranslationCall],
    index: &CatalogIndex,
    locale: Option<&str>,
    range: Range,
) -> Vec<InlayHint> {
    let Some(locale) = locale else { return vec![] };

    calls
        .iter()
        .filter(|c| c.msgid.is_some() && ranges_overlap(c.range, range))
        .filter_map(|call| {
            let msgid = call.msgid.as_deref()?;
            let key = CatalogKey { msgid: msgid.into(), msgctxt: call.msgctxt.clone() };
            let label = resolve_label(index, &key, locale)?;
            Some(InlayHint {
                position: call.range.end,
                label: InlayHintLabel::String(label),
                kind: Some(InlayHintKind::PARAMETER),
                text_edits: None,
                tooltip: None,
                padding_left: None,
                padding_right: None,
                data: None,
            })
        })
        .collect()
}

/// Build the hint label for a key+locale, or `None` if the locale has no entry.
///
/// Returns `None` when the msgid is not in the index at all for this locale
/// (so we don't hint calls that the catalog doesn't know about). When the
/// entry exists but `msgstr` is empty → ` = (untranslated)` (REQ-HINT-06).
/// When the entry is fuzzy → appends ` (fuzzy)` (REQ-HINT-07).
fn resolve_label(index: &CatalogIndex, key: &CatalogKey, locale: &str) -> Option<String> {
    let entry = index
        .lookup(key)
        .iter()
        .find(|e| e.locale == locale && !e.flags.obsolete)?;

    let text = if entry.msgstr.iter().all(|s| s.is_empty()) {
        "(untranslated)".to_string()
    } else {
        let raw = entry.msgstr[0].as_str();
        truncate(raw)
    };

    let suffix = if entry.flags.fuzzy { " (fuzzy)" } else { "" };
    Some(format!(" = {text}{suffix}"))
}

fn truncate(s: &str) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(TRUNCATE_CHARS).collect();
    if chars.next().is_some() { format!("{head}…") } else { head }
}

fn ranges_overlap(call: Range, viewport: Range) -> bool {
    call.start <= viewport.end && call.end >= viewport.start
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tower_lsp_server::ls_types::{Position, Range};

    use super::*;
    use crate::catalog::index::{CatalogEntry, EntryFlags};
    use crate::extract::types::TranslationFunc;

    fn pos(line: u32, ch: u32) -> Position {
        Position { line, character: ch }
    }

    fn rng(sl: u32, sc: u32, el: u32, ec: u32) -> Range {
        Range { start: pos(sl, sc), end: pos(el, ec) }
    }

    fn make_entry(locale: &str, msgid: &str, msgstr: &str) -> CatalogEntry {
        CatalogEntry {
            locale: locale.into(),
            domain: "messages".into(),
            msgid: msgid.into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec![msgstr.into()],
            flags: EntryFlags { fuzzy: false, obsolete: false },
            file_path: PathBuf::from("/locale/de/messages.po"),
            line: 1,
        }
    }

    fn make_call(msgid: &str, call_range: Range) -> TranslationCall {
        TranslationCall {
            func: TranslationFunc::Gettext,
            msgid: Some(msgid.into()),
            msgid_plural: None,
            msgctxt: None,
            domain: None,
            range: call_range,
            msgid_range: None,
            unresolved_reason: None,
            unresolved_arg_range: None,
            is_implicit_concat: false,
        }
    }

    fn full_range() -> Range {
        rng(0, 0, 999, 999)
    }

    // REQ-HINT-01 — no locale → no hints

    #[test]
    fn req_hint_01_no_locale_returns_empty() {
        let index = CatalogIndex::build(vec![make_entry("de", "Checkout", "Kasse")]);
        let calls = vec![make_call("Checkout", rng(0, 0, 0, 22))];
        let hints = inlay_hints(&calls, &index, None, full_range());
        assert!(hints.is_empty());
    }

    // REQ-HINT-02 — one hint per resolved call

    #[test]
    fn req_hint_02_hint_for_translated_call() {
        let index = CatalogIndex::build(vec![make_entry("de", "Checkout", "Kasse")]);
        let calls = vec![make_call("Checkout", rng(0, 0, 0, 22))];
        let hints = inlay_hints(&calls, &index, Some("de"), full_range());
        assert_eq!(hints.len(), 1);
        assert!(matches!(&hints[0].label, InlayHintLabel::String(s) if s == " = Kasse"));
    }

    #[test]
    fn req_hint_02_no_hint_when_locale_absent() {
        let index = CatalogIndex::build(vec![make_entry("de", "Checkout", "Kasse")]);
        let calls = vec![make_call("Checkout", rng(0, 0, 0, 22))];
        let hints = inlay_hints(&calls, &index, Some("fr"), full_range());
        assert!(hints.is_empty());
    }

    #[test]
    fn req_hint_02_no_hint_for_unknown_msgid() {
        let index = CatalogIndex::build(vec![]);
        let calls = vec![make_call("Unknown", rng(0, 0, 0, 22))];
        let hints = inlay_hints(&calls, &index, Some("de"), full_range());
        assert!(hints.is_empty());
    }

    // REQ-HINT-03 — hint position and format

    #[test]
    fn req_hint_03_position_is_end_of_call() {
        let index = CatalogIndex::build(vec![make_entry("de", "Checkout", "Kasse")]);
        let calls = vec![make_call("Checkout", rng(5, 10, 5, 32))];
        let hints = inlay_hints(&calls, &index, Some("de"), full_range());
        assert_eq!(hints[0].position, pos(5, 32));
    }

    #[test]
    fn req_hint_03_label_starts_with_eq() {
        let index = CatalogIndex::build(vec![make_entry("de", "Checkout", "Kasse")]);
        let calls = vec![make_call("Checkout", rng(0, 0, 0, 5))];
        let hints = inlay_hints(&calls, &index, Some("de"), full_range());
        let label = match &hints[0].label {
            InlayHintLabel::String(s) => s.clone(),
            _ => panic!("expected string label"),
        };
        assert!(label.starts_with(" = "), "label: {label:?}");
    }

    #[test]
    fn req_hint_03_kind_is_parameter() {
        let index = CatalogIndex::build(vec![make_entry("de", "Checkout", "Kasse")]);
        let calls = vec![make_call("Checkout", rng(0, 0, 0, 5))];
        let hints = inlay_hints(&calls, &index, Some("de"), full_range());
        assert_eq!(hints[0].kind, Some(InlayHintKind::PARAMETER));
    }

    // REQ-HINT-03 — range filtering

    #[test]
    fn req_hint_03_only_calls_in_range_get_hints() {
        let index = CatalogIndex::build(vec![
            make_entry("de", "A", "Eins"),
            make_entry("de", "B", "Zwei"),
        ]);
        let calls = vec![
            make_call("A", rng(1, 0, 1, 10)),  // in range
            make_call("B", rng(20, 0, 20, 10)), // outside
        ];
        let viewport = rng(0, 0, 5, 0);
        let hints = inlay_hints(&calls, &index, Some("de"), viewport);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].position, pos(1, 10));
    }

    // REQ-HINT-04 — truncation

    #[test]
    fn req_hint_04_long_translation_is_truncated() {
        let long = "A".repeat(60);
        let index = CatalogIndex::build(vec![make_entry("de", "Key", &long)]);
        let calls = vec![make_call("Key", rng(0, 0, 0, 5))];
        let hints = inlay_hints(&calls, &index, Some("de"), full_range());
        let label = match &hints[0].label {
            InlayHintLabel::String(s) => s.clone(),
            _ => panic!(),
        };
        assert!(label.contains('…'), "should contain ellipsis: {label:?}");
        assert!(label.chars().count() < 60, "should be shorter than original");
    }

    #[test]
    fn req_hint_04_short_translation_not_truncated() {
        let index = CatalogIndex::build(vec![make_entry("de", "Checkout", "Kasse")]);
        let calls = vec![make_call("Checkout", rng(0, 0, 0, 5))];
        let hints = inlay_hints(&calls, &index, Some("de"), full_range());
        let label = match &hints[0].label {
            InlayHintLabel::String(s) => s.clone(),
            _ => panic!(),
        };
        assert!(!label.contains('…'));
    }

    // REQ-HINT-06 — untranslated

    #[test]
    fn req_hint_06_empty_msgstr_shows_untranslated() {
        let index = CatalogIndex::build(vec![make_entry("de", "Checkout", "")]);
        let calls = vec![make_call("Checkout", rng(0, 0, 0, 5))];
        let hints = inlay_hints(&calls, &index, Some("de"), full_range());
        assert_eq!(hints.len(), 1);
        assert!(
            matches!(&hints[0].label, InlayHintLabel::String(s) if s.contains("untranslated")),
            "got: {:?}", hints[0].label
        );
    }

    // REQ-HINT-07 — fuzzy

    #[test]
    fn req_hint_07_fuzzy_entry_shows_fuzzy_marker() {
        let mut entry = make_entry("de", "Save", "Speichern");
        entry.flags.fuzzy = true;
        let index = CatalogIndex::build(vec![entry]);
        let calls = vec![make_call("Save", rng(0, 0, 0, 5))];
        let hints = inlay_hints(&calls, &index, Some("de"), full_range());
        assert_eq!(hints.len(), 1);
        assert!(
            matches!(&hints[0].label, InlayHintLabel::String(s) if s.contains("fuzzy")),
            "got: {:?}", hints[0].label
        );
    }

    #[test]
    fn obsolete_entry_yields_no_hint() {
        let mut entry = make_entry("de", "Old", "Alt");
        entry.flags.obsolete = true;
        let index = CatalogIndex::build(vec![entry]);
        let calls = vec![make_call("Old", rng(0, 0, 0, 5))];
        let hints = inlay_hints(&calls, &index, Some("de"), full_range());
        assert!(hints.is_empty());
    }

    #[test]
    fn unresolved_call_yields_no_hint() {
        let index = CatalogIndex::build(vec![make_entry("de", "Checkout", "Kasse")]);
        let mut call = make_call("Checkout", rng(0, 0, 0, 5));
        call.msgid = None;
        let hints = inlay_hints(&[call], &index, Some("de"), full_range());
        assert!(hints.is_empty());
    }
}
