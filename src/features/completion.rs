use ropey::Rope;
use tower_lsp_server::ls_types::*;

use crate::catalog::index::{CatalogIndex, CatalogKey};
use crate::extract::types::TranslationCall;
use crate::util::{PositionEncoding, char_offset_to_lsp_pos, lsp_pos_to_char_offset, pos_in_range};

/// Build completion items for a msgid position in a translation call.
///
/// `calls` is the pre-extracted list for the current document (caller picks
/// the right extractor — Python vs Jinja — based on the file extension).
pub fn complete(
    rope: &Rope,
    calls: &[TranslationCall],
    pos: Position,
    enc: PositionEncoding,
    index: &CatalogIndex,
) -> Vec<CompletionItem> {
    // REQ-CPL-01: cursor must sit inside the msgid string literal of a call.
    // msgid_range is always inside call.range, so checking call.range is redundant.
    let call = calls
        .iter()
        .find(|c| c.msgid_range.is_some_and(|r| pos_in_range(pos, r)));

    let Some(call) = call else {
        return vec![];
    };

    let msgid_range = call.msgid_range.unwrap();

    // Extract the partial text typed between the opening quote and the cursor.
    let Some((prefix, prefix_start)) = string_prefix(rope, msgid_range.start, pos, enc) else {
        return vec![];
    };

    // REQ-CPL-04: pgettext context biases sorting.
    let msgctxt = call.msgctxt.as_deref();

    // REQ-CPL-03: prefix matches first, then contains matches.
    let prefix_lower = prefix.to_lowercase();
    let mut candidates: Vec<(&CatalogKey, bool)> = index
        .all_msgids()
        .filter(|key| prefix_lower.is_empty() || key.msgid.to_lowercase().contains(&prefix_lower))
        .map(|key| {
            let is_prefix = key.msgid.to_lowercase().starts_with(&prefix_lower);
            (key, is_prefix)
        })
        .collect();

    candidates.sort_by(|(a, a_prefix), (b, b_prefix)| {
        let ctx_score = |key: &CatalogKey| -> bool {
            msgctxt.is_some_and(|ctx| key.msgctxt.as_deref() == Some(ctx))
        };
        ctx_score(b)
            .cmp(&ctx_score(a))
            .then(b_prefix.cmp(a_prefix))
            .then(a.msgid.cmp(&b.msgid))
    });

    // REQ-CPL-05/06: build items with label, detail, text_edit, and optional table.
    let prefix_range = Range {
        start: prefix_start,
        end: pos,
    };

    // Cap short/empty-prefix results to avoid O(N) documentation table builds for
    // large catalogs. When the prefix is very short we skip per-item documentation
    // and limit to 100 items so the list stays responsive.
    let short_prefix = prefix_lower.len() < 2;
    let iter: Box<dyn Iterator<Item = _>> = if short_prefix {
        Box::new(candidates.iter().enumerate().take(100))
    } else {
        Box::new(candidates.iter().enumerate())
    };

    iter.map(|(idx, (key, _))| {
        if short_prefix {
            build_item_no_docs(key, index, prefix_range, idx)
        } else {
            build_item(key, index, prefix_range, idx)
        }
    })
    .collect()
}

fn build_item(
    key: &CatalogKey,
    index: &CatalogIndex,
    prefix_range: Range,
    sort_index: usize,
) -> CompletionItem {
    let mut entries: Vec<_> = index.lookup(key).iter().collect();

    // Sort deterministically by locale then domain before any use.
    entries.sort_by(|a, b| a.locale.cmp(&b.locale).then(a.domain.cmp(&b.domain)));

    // REQ-CPL-05: detail = first locale's translation status.
    let detail = entries.first().map(|e| {
        let msgstr = e.msgstr.iter().find(|s| !s.is_empty());
        if let Some(s) = msgstr {
            format!("[{}] {}", e.locale, s)
        } else {
            format!("[{}] (untranslated)", e.locale)
        }
    });

    // REQ-CPL-06: multi-locale table when more than one entry exists.
    let documentation = if entries.len() > 1 {
        let rows: String = entries
            .iter()
            .map(|e| {
                let msgstr = e
                    .msgstr
                    .iter()
                    .find(|s| !s.is_empty())
                    .map(|s| s.as_str())
                    .unwrap_or("_(untranslated)_");
                format!("| {} | {} |\n", e.locale, msgstr)
            })
            .collect();
        let table = format!("| Locale | Translation |\n|--------|-------------|\n{rows}");
        Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: table,
        }))
    } else {
        None
    };

    CompletionItem {
        label: key.msgid.clone(),
        kind: Some(CompletionItemKind::TEXT),
        detail,
        documentation,
        // REQ-CPL-03: filter_text lets the client match contains-only items against the
        // typed prefix (the label may not start with the typed text).
        filter_text: Some(key.msgid.clone()),
        // Preserve server-side ordering; 4-digit zero-padded index keeps lexicographic sort
        // consistent with the ranked candidate list.
        sort_text: Some(format!("{:04}", sort_index)),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range: prefix_range,
            new_text: key.msgid.clone(),
        })),
        ..Default::default()
    }
}

/// Like `build_item` but skips building the documentation table. Used when the
/// typed prefix is very short (< 2 chars) to avoid expensive O(N) work.
fn build_item_no_docs(
    key: &CatalogKey,
    index: &CatalogIndex,
    prefix_range: Range,
    sort_index: usize,
) -> CompletionItem {
    let mut entries: Vec<_> = index.lookup(key).iter().collect();
    entries.sort_by(|a, b| a.locale.cmp(&b.locale).then(a.domain.cmp(&b.domain)));

    let detail = entries.first().map(|e| {
        let msgstr = e.msgstr.iter().find(|s| !s.is_empty());
        if let Some(s) = msgstr {
            format!("[{}] {}", e.locale, s)
        } else {
            format!("[{}] (untranslated)", e.locale)
        }
    });

    CompletionItem {
        label: key.msgid.clone(),
        kind: Some(CompletionItemKind::TEXT),
        detail,
        documentation: None,
        filter_text: Some(key.msgid.clone()),
        sort_text: Some(format!("{:04}", sort_index)),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range: prefix_range,
            new_text: key.msgid.clone(),
        })),
        ..Default::default()
    }
}

/// Given the start position of a string node (pointing at the opening quote /
/// string prefix), scan forward to find where the string content begins, then
/// return the text typed between that content start and `cursor` together with
/// the LSP position of the content start.
fn string_prefix(
    rope: &Rope,
    string_start: Position,
    cursor: Position,
    enc: PositionEncoding,
) -> Option<(String, Position)> {
    let start_offset = lsp_pos_to_char_offset(rope, string_start, enc);
    let cursor_offset = lsp_pos_to_char_offset(rope, cursor, enc);

    if cursor_offset <= start_offset {
        return None;
    }

    // Text from the start of the string node up to the cursor.
    let head: String = rope.slice(start_offset..cursor_offset).chars().collect();

    // Skip any string prefix characters (r, u, b and their upper-case forms).
    let mut skip = 0usize;
    for c in head.chars() {
        if matches!(c, 'r' | 'R' | 'u' | 'U' | 'b' | 'B') {
            skip += c.len_utf8();
        } else {
            break;
        }
    }

    let after_prefix = &head[skip..];

    // Identify the opening quote sequence.
    let quote_byte_len = if after_prefix.starts_with("\"\"\"") || after_prefix.starts_with("'''") {
        3usize
    } else if after_prefix.starts_with('"') || after_prefix.starts_with('\'') {
        1usize
    } else {
        return None;
    };

    // Everything after the opening quote(s) up to the first closing quote is the
    // typed prefix. If the cursor is positioned after the closing quote we must
    // stop there to avoid including the close quote (or text beyond it) in the
    // prefix, which would corrupt match results.
    let raw_content = &after_prefix[quote_byte_len..];
    let content = raw_content
        .split(['"', '\''])
        .next()
        .unwrap_or(raw_content)
        .to_string();

    // Calculate the LSP position right after the opening quote(s).
    let content_byte_start = skip + quote_byte_len;
    let content_char_start = head[..content_byte_start].chars().count();
    let content_lsp_start = char_offset_to_lsp_pos(rope, start_offset + content_char_start, enc);

    Some((content, content_lsp_start))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::catalog::index::{CatalogEntry, CatalogIndex, EntryFlags};
    use crate::extract::python;
    use crate::extract::types::TranslationFunc;

    fn no_extra() -> HashMap<String, TranslationFunc> {
        HashMap::new()
    }

    // ── Test index helpers ────────────────────────────────────────────────────

    fn entry(locale: &str, msgid: &str, msgstr: &str) -> CatalogEntry {
        CatalogEntry {
            locale: locale.into(),
            domain: "messages".into(),
            msgid: msgid.into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec![msgstr.into()],
            flags: EntryFlags {
                fuzzy: false,
                obsolete: false,
            },
            file_path: PathBuf::from("/locale/messages.po"),
            line: 1,
        }
    }

    fn entry_with_ctx(locale: &str, msgid: &str, msgctxt: &str, msgstr: &str) -> CatalogEntry {
        let mut e = entry(locale, msgid, msgstr);
        e.msgctxt = Some(msgctxt.into());
        e
    }

    /// Build a simple index with de/fr translations for clean-shopfront data.
    fn shopfront_index() -> CatalogIndex {
        CatalogIndex::build(vec![
            entry("de", "Checkout", "Kasse"),
            entry("fr", "Checkout", ""),
            entry("de", "Save", "Speichern"),
        ])
    }

    /// Call complete() for a Python source at the given (0-based) line/character.
    fn complete_at(
        src: &str,
        line: u32,
        character: u32,
        index: &CatalogIndex,
    ) -> Vec<CompletionItem> {
        let rope = Rope::from_str(src);
        let calls = python::extract(src.as_bytes(), &no_extra());
        complete(
            &rope,
            &calls,
            Position { line, character },
            PositionEncoding::Utf16,
            index,
        )
    }

    // ── REQ-CPL-01 ───────────────────────────────────────────────────────────

    #[test]
    fn req_cpl_01_completes_only_in_msgid_position() {
        let index = shopfront_index();
        // Cursor inside the msgid string → items returned.
        let items = complete_at(r#"_("Che")"#, 0, 5, &index);
        assert!(!items.is_empty());

        // Cursor outside any call → empty.
        let items = complete_at(r#"x = 1"#, 0, 3, &index);
        assert!(items.is_empty());

        // Cursor past end of call (col 8 = after `)`) → empty.
        let items = complete_at(r#"_("Che")"#, 0, 8, &index);
        assert!(items.is_empty());
    }

    // ── REQ-CPL-03 ───────────────────────────────────────────────────────────

    #[test]
    fn req_cpl_03_prefix_before_contains() {
        // "Che" → "Checkout" starts with "Che" (prefix match).
        // Add a key that only contains "hec" to verify ordering.
        let index = CatalogIndex::build(vec![
            entry("de", "Checkout", "Kasse"),
            entry("de", "The checkout process", ""),
        ]);

        let items = complete_at(r#"_("Che")"#, 0, 5, &index);

        // Prefix match ("Checkout") sorts before contains-only ("The checkout process").
        assert_eq!(items[0].label, "Checkout");
        assert_eq!(items[1].label, "The checkout process");
    }

    #[test]
    fn req_cpl_03_empty_prefix_returns_all() {
        let index = shopfront_index();
        // Cursor right after opening quote → empty prefix → all msgids.
        let items = complete_at(r#"_("")"#, 0, 3, &index);
        assert_eq!(items.len(), 2); // Checkout and Save
    }

    #[test]
    fn req_cpl_03_no_match_returns_empty() {
        let index = shopfront_index();
        let items = complete_at(r#"_("Zzz")"#, 0, 5, &index);
        assert!(items.is_empty());
    }

    #[test]
    fn req_cpl_03_empty_index_returns_empty() {
        let index = CatalogIndex::default();
        let items = complete_at(r#"_("Che")"#, 0, 5, &index);
        assert!(items.is_empty());
    }

    // ── REQ-CPL-04 ───────────────────────────────────────────────────────────

    #[test]
    fn req_cpl_04_prefers_msgctxt_in_pgettext() {
        let index = CatalogIndex::build(vec![
            entry("de", "Save", "Speichern"),                  // plain key
            entry_with_ctx("de", "Save", "button", "Sichern"), // context key
        ]);

        // pgettext("button", "Sa") — context = "button"
        let items = complete_at(r#"pgettext("button", "Sa")"#, 0, 21, &index);

        // Context-matching key ("button" Save) must sort first.
        assert_eq!(items[0].label, "Save");
        // Verify the first item corresponds to the context-tagged entry.
        assert!(items[0].detail.as_deref().unwrap_or("").contains("Sichern"));
    }

    // ── REQ-CPL-05 ───────────────────────────────────────────────────────────

    #[test]
    fn req_cpl_05_item_label_detail_and_text_edit() {
        let index = shopfront_index();
        // _("Che") — cursor at col 5 (inside "Che")
        let items = complete_at(r#"_("Che")"#, 0, 5, &index);

        let checkout = items.iter().find(|i| i.label == "Checkout").unwrap();
        // kind = TEXT
        assert_eq!(checkout.kind, Some(CompletionItemKind::TEXT));
        // detail shows first locale translation
        assert!(checkout.detail.as_deref().unwrap_or("").starts_with("[de]"));
        // text_edit replaces "Ch" (cols 3–5, cursor at col 5 = after 'h' before 'e')
        let edit = match checkout.text_edit.as_ref().unwrap() {
            CompletionTextEdit::Edit(e) => e,
            _ => panic!("expected Edit"),
        };
        // start = right after the opening `"` (col 2), so col 3
        assert_eq!(
            edit.range.start,
            Position {
                line: 0,
                character: 3
            }
        );
        // end = cursor position (col 5)
        assert_eq!(
            edit.range.end,
            Position {
                line: 0,
                character: 5
            }
        );
        assert_eq!(edit.new_text, "Checkout");
    }

    // ── REQ-CPL-06 ───────────────────────────────────────────────────────────

    #[test]
    fn req_cpl_06_multi_locale_documentation_table() {
        let index = shopfront_index(); // Checkout: de="Kasse", fr="" (untranslated)
        let items = complete_at(r#"_("Che")"#, 0, 5, &index);
        let checkout = items.iter().find(|i| i.label == "Checkout").unwrap();

        // Two locales → markdown table in documentation.
        let doc = checkout
            .documentation
            .as_ref()
            .expect("documentation present");
        let markdown = match doc {
            Documentation::MarkupContent(mc) => &mc.value,
            _ => panic!("expected MarkupContent"),
        };
        assert!(
            markdown.contains("| Locale | Translation |"),
            "table header missing"
        );
        assert!(markdown.contains("de"), "de locale missing");
        assert!(markdown.contains("fr"), "fr locale missing");
        assert!(
            markdown.contains("_(untranslated)_"),
            "untranslated marker missing"
        );
    }

    #[test]
    fn req_cpl_06_single_locale_no_table() {
        let index = CatalogIndex::build(vec![entry("de", "Checkout", "Kasse")]);
        let items = complete_at(r#"_("Che")"#, 0, 5, &index);
        let checkout = items.iter().find(|i| i.label == "Checkout").unwrap();
        // Only one locale → no documentation table.
        assert!(checkout.documentation.is_none());
    }

    // ── string_prefix edge cases ──────────────────────────────────────────────

    #[test]
    fn string_prefix_triple_quote() {
        let rope = Rope::from_str(r#""""Checkout""""#);
        let start = Position {
            line: 0,
            character: 0,
        };
        let cursor = Position {
            line: 0,
            character: 6,
        };
        let (prefix, prefix_start) =
            string_prefix(&rope, start, cursor, PositionEncoding::Utf16).unwrap();
        assert_eq!(prefix, "Che");
        assert_eq!(
            prefix_start,
            Position {
                line: 0,
                character: 3
            }
        );
    }

    #[test]
    fn string_prefix_with_r_prefix() {
        let rope = Rope::from_str(r#"r"Checkout""#);
        let start = Position {
            line: 0,
            character: 0,
        };
        let cursor = Position {
            line: 0,
            character: 5,
        };
        let (prefix, prefix_start) =
            string_prefix(&rope, start, cursor, PositionEncoding::Utf16).unwrap();
        assert_eq!(prefix, "Che");
        assert_eq!(
            prefix_start,
            Position {
                line: 0,
                character: 2
            }
        ); // after r"
    }

    #[test]
    fn pos_in_range_basic() {
        let range = Range {
            start: Position {
                line: 0,
                character: 3,
            },
            end: Position {
                line: 0,
                character: 10,
            },
        };
        assert!(pos_in_range(
            Position {
                line: 0,
                character: 3
            },
            range
        ));
        assert!(pos_in_range(
            Position {
                line: 0,
                character: 7
            },
            range
        ));
        assert!(!pos_in_range(
            Position {
                line: 0,
                character: 10
            },
            range
        )); // exclusive end
        assert!(!pos_in_range(
            Position {
                line: 0,
                character: 2
            },
            range
        ));
    }

    // ── babel-lsp-6ke: closing quote must not be included in prefix ───────────

    #[test]
    fn string_prefix_cursor_after_close_quote_strips_close_quote() {
        // Cursor is positioned one past the closing quote (col 11 in `"Checkout"`).
        // The prefix must be "Checkout" without the closing `"`.
        let rope = Rope::from_str(r#""Checkout""#);
        let start = Position {
            line: 0,
            character: 0,
        };
        let cursor = Position {
            line: 0,
            character: 10,
        }; // past the closing `"`
        let (prefix, _) = string_prefix(&rope, start, cursor, PositionEncoding::Utf16).unwrap();
        assert_eq!(prefix, "Checkout");
    }

    // ── babel-lsp-1hs: empty-prefix results are capped at 100, no docs table ──

    #[test]
    fn empty_prefix_capped_at_100_and_no_documentation() {
        // Build an index with 150 distinct msgids.
        let entries: Vec<_> = (0..150u32)
            .map(|i| entry("de", &format!("Key{:03}", i), "Übersetzung"))
            .collect();
        let index = CatalogIndex::build(entries);

        // Cursor right after opening quote → empty prefix.
        let items = complete_at(r#"_("")"#, 0, 3, &index);

        assert!(
            items.len() <= 100,
            "expected at most 100 items, got {}",
            items.len()
        );
        for item in &items {
            assert!(
                item.documentation.is_none(),
                "no documentation expected for empty-prefix items, but '{}' has one",
                item.label
            );
        }
    }

    #[test]
    fn non_empty_prefix_two_chars_includes_documentation() {
        // With a prefix of 2+ chars, full docs should be built.
        let index = shopfront_index(); // Checkout: de + fr
        let items = complete_at(r#"_("Ch")"#, 0, 5, &index);
        let checkout = items.iter().find(|i| i.label == "Checkout").unwrap();
        // Two locales → documentation table present.
        assert!(checkout.documentation.is_some());
    }
}
