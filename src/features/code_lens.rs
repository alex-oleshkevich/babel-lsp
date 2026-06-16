use serde::{Deserialize, Serialize};
use tower_lsp_server::ls_types::{CodeLens, Command, Position, Range};

use crate::catalog::index::{CatalogEntry, CatalogIndex, CatalogKey};
use crate::catalog::loader::PoLineMap;
use crate::extract::types::TranslationCall;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LensKind {
    Catalog,
    Source,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LensData {
    pub key: CatalogKey,
    pub kind: LensKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

/// Build lazy code lenses for a catalog (`.po`/`.pot`) file.
///
/// Returns one lens per distinct msgid, with `command: None` — the visible title
/// is filled in later by [`resolve_lens`] (codeLens/resolve).
pub fn code_lenses_catalog(source: &str, entries: &[&CatalogEntry]) -> Vec<CodeLens> {
    let line_map = PoLineMap::build(source);
    let mut seen = std::collections::HashSet::new();
    let mut lenses = Vec::new();

    for entry in entries {
        let key = entry.key();
        if !seen.insert(key.clone()) {
            continue;
        }
        let Some(line) = line_map.get_line(&key) else {
            continue;
        };
        // PoLineMap is 1-based; LSP positions are 0-based
        let pos = Position {
            line: line.saturating_sub(1),
            character: 0,
        };
        let locale = if entry.locale.is_empty() {
            None
        } else {
            Some(entry.locale.clone())
        };
        let data = LensData {
            key,
            kind: LensKind::Catalog,
            locale,
        };
        lenses.push(CodeLens {
            range: Range { start: pos, end: pos },
            command: None,
            data: serde_json::to_value(&data).ok(),
        });
    }

    lenses
}

/// Build lazy code lenses for a source file (Python / Jinja).
///
/// Returns one lens per resolved translation call, anchored to the msgid literal.
pub fn code_lenses_source(calls: &[TranslationCall]) -> Vec<CodeLens> {
    calls
        .iter()
        .filter_map(|call| {
            let msgid = call.msgid.as_deref()?;
            let key = CatalogKey {
                msgid: msgid.to_owned(),
                msgctxt: call.msgctxt.clone(),
            };
            let range = call.msgid_range.unwrap_or(call.range);
            let data = LensData {
                key,
                kind: LensKind::Source,
                locale: None,
            };
            Some(CodeLens {
                range,
                command: None,
                data: serde_json::to_value(&data).ok(),
            })
        })
        .collect()
}

/// Fill in the command title and arguments for a lazy lens (codeLens/resolve).
///
/// - Catalog lens → `"k of m locales translated"` (plus `"· fuzzy"` if the entry
///   for the stored locale carries the fuzzy flag).
/// - Source lens  → `"used N time(s)"`, counting calls in `all_source_calls`.
///
/// The command is always `babel-lsp.findReferences` with the serialised key as
/// its first argument.
pub fn resolve_lens(
    lens: CodeLens,
    index: &CatalogIndex,
    all_source_calls: &[TranslationCall],
) -> CodeLens {
    let Some(ref data_val) = lens.data else {
        return lens;
    };
    let Ok(data) = serde_json::from_value::<LensData>(data_val.clone()) else {
        return lens;
    };

    let title = match data.kind {
        LensKind::Catalog => {
            let m = index.all_locales().len();
            let missing = index.missing_locales(&data.key).len();
            let k = m.saturating_sub(missing);
            let is_fuzzy = data
                .locale
                .as_deref()
                .map(|locale| {
                    index
                        .lookup(&data.key)
                        .iter()
                        .any(|e| e.locale == locale && e.flags.fuzzy)
                })
                .unwrap_or(false);
            let mut t = format!("{k} of {m} locales translated");
            if is_fuzzy {
                t.push_str(" · fuzzy");
            }
            t
        }
        LensKind::Source => {
            let count = all_source_calls
                .iter()
                .filter(|c| {
                    c.msgid.as_deref() == Some(data.key.msgid.as_str())
                        && c.msgctxt == data.key.msgctxt
                })
                .count();
            if count == 1 {
                "used 1 time".to_owned()
            } else {
                format!("used {count} times")
            }
        }
    };

    let key_json = serde_json::to_value(&data.key).unwrap_or(serde_json::Value::Null);
    let command = Command {
        title,
        command: "babel-lsp.findReferences".to_owned(),
        arguments: Some(vec![key_json]),
    };

    CodeLens {
        range: lens.range,
        command: Some(command),
        data: lens.data,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tower_lsp_server::ls_types::{Position, Range};

    use super::*;
    use crate::catalog::index::{CatalogEntry, CatalogIndex, EntryFlags};
    use crate::extract::types::TranslationFunc;

    fn make_entry(msgid: &str, locale: &str, fuzzy: bool) -> CatalogEntry {
        CatalogEntry {
            locale: locale.to_owned(),
            domain: "messages".to_owned(),
            msgid: msgid.to_owned(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: if !fuzzy && !locale.is_empty() {
                vec![format!("[{locale}] {msgid}")]
            } else {
                vec![]
            },
            flags: EntryFlags { fuzzy, obsolete: false },
            file_path: PathBuf::from(format!("locale/{locale}/LC_MESSAGES/messages.po")),
            line: 1,
        }
    }

    fn make_call(msgid: &str, line: u32) -> TranslationCall {
        let pos = Position { line, character: 4 };
        let end_pos = Position { line, character: 4 + msgid.len() as u32 };
        TranslationCall {
            func: TranslationFunc::Gettext,
            msgid: Some(msgid.to_owned()),
            msgid_plural: None,
            msgctxt: None,
            domain: None,
            range: Range { start: pos, end: end_pos },
            msgid_range: Some(Range { start: pos, end: end_pos }),
            unresolved_reason: None,
            unresolved_arg_range: None,
            is_implicit_concat: false,
        }
    }

    fn make_catalog_lens(key: CatalogKey, locale: Option<&str>) -> CodeLens {
        let data = LensData {
            key,
            kind: LensKind::Catalog,
            locale: locale.map(str::to_owned),
        };
        CodeLens {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 0 },
            },
            command: None,
            data: serde_json::to_value(&data).ok(),
        }
    }

    fn make_source_lens(key: CatalogKey) -> CodeLens {
        let data = LensData { key, kind: LensKind::Source, locale: None };
        CodeLens {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 0 },
            },
            command: None,
            data: serde_json::to_value(&data).ok(),
        }
    }

    // ── catalog lenses ────────────────────────────────────────────────────────

    #[test]
    fn catalog_lens_emitted_per_entry() {
        let po = "msgid \"Hello\"\nmsgstr \"Hi\"\n\nmsgid \"World\"\nmsgstr \"Mundo\"\n";
        let e1 = make_entry("Hello", "es", false);
        let e2 = make_entry("World", "es", false);
        let lenses = code_lenses_catalog(po, &[&e1, &e2]);
        assert_eq!(lenses.len(), 2);
    }

    #[test]
    fn catalog_lens_no_command_before_resolve() {
        let po = "msgid \"Hello\"\nmsgstr \"Hi\"\n";
        let entry = make_entry("Hello", "es", false);
        let lenses = code_lenses_catalog(po, &[&entry]);
        assert!(lenses[0].command.is_none());
    }

    #[test]
    fn catalog_lens_range_at_msgid_line() {
        // msgid is on line 2 (1-based) → LSP line 1 (0-based)
        let po = "# comment\nmsgid \"Hello\"\nmsgstr \"Hi\"\n";
        let entry = make_entry("Hello", "es", false);
        let lenses = code_lenses_catalog(po, &[&entry]);
        assert_eq!(lenses[0].range.start.line, 1);
    }

    #[test]
    fn catalog_lens_deduplicates_same_key() {
        let po = "msgid \"Hello\"\nmsgstr \"Hi\"\n";
        let e1 = make_entry("Hello", "es", false);
        let e2 = make_entry("Hello", "fr", false);
        let lenses = code_lenses_catalog(po, &[&e1, &e2]);
        assert_eq!(lenses.len(), 1);
    }

    #[test]
    fn catalog_lens_data_round_trips() {
        let po = "msgid \"Hello\"\nmsgstr \"Hi\"\n";
        let entry = make_entry("Hello", "es", false);
        let lenses = code_lenses_catalog(po, &[&entry]);
        let data: LensData = serde_json::from_value(lenses[0].data.clone().unwrap()).unwrap();
        assert_eq!(data.key.msgid, "Hello");
        assert_eq!(data.kind, LensKind::Catalog);
        assert_eq!(data.locale.as_deref(), Some("es"));
    }

    // ── source lenses ─────────────────────────────────────────────────────────

    #[test]
    fn source_lens_emitted_per_resolved_call() {
        let calls = vec![make_call("Hello", 0), make_call("World", 2)];
        let lenses = code_lenses_source(&calls);
        assert_eq!(lenses.len(), 2);
    }

    #[test]
    fn source_lens_skips_unresolved_calls() {
        let mut call = make_call("", 0);
        call.msgid = None;
        let lenses = code_lenses_source(&[call]);
        assert!(lenses.is_empty());
    }

    #[test]
    fn source_lens_no_command_before_resolve() {
        let calls = vec![make_call("Hello", 0)];
        let lenses = code_lenses_source(&calls);
        assert!(lenses[0].command.is_none());
    }

    #[test]
    fn source_lens_data_round_trips() {
        let calls = vec![make_call("Hello", 0)];
        let lenses = code_lenses_source(&calls);
        let data: LensData = serde_json::from_value(lenses[0].data.clone().unwrap()).unwrap();
        assert_eq!(data.kind, LensKind::Source);
        assert_eq!(data.key.msgid, "Hello");
    }

    // ── resolve: catalog ──────────────────────────────────────────────────────

    #[test]
    fn resolve_catalog_shows_full_coverage() {
        let entries = vec![make_entry("Hello", "es", false), make_entry("Hello", "fr", false)];
        let index = CatalogIndex::build(entries);
        let lens = make_catalog_lens(CatalogKey::new("Hello"), Some("es"));
        let cmd = resolve_lens(lens, &index, &[]).command.unwrap();
        assert_eq!(cmd.title, "2 of 2 locales translated");
    }

    #[test]
    fn resolve_catalog_counts_missing() {
        // fr has empty msgstr → treated as missing by missing_locales
        let entries = vec![make_entry("Hello", "es", false), make_entry("Hello", "fr", true)];
        let index = CatalogIndex::build(entries);
        let lens = make_catalog_lens(CatalogKey::new("Hello"), Some("es"));
        let cmd = resolve_lens(lens, &index, &[]).command.unwrap();
        assert_eq!(cmd.title, "1 of 2 locales translated");
    }

    #[test]
    fn resolve_catalog_appends_fuzzy() {
        let entries = vec![make_entry("Hello", "es", true)];
        let index = CatalogIndex::build(entries);
        let lens = make_catalog_lens(CatalogKey::new("Hello"), Some("es"));
        let cmd = resolve_lens(lens, &index, &[]).command.unwrap();
        assert!(cmd.title.contains("· fuzzy"));
    }

    #[test]
    fn resolve_catalog_fuzzy_only_for_stored_locale() {
        let entries = vec![make_entry("Hello", "es", true), make_entry("Hello", "fr", false)];
        let index = CatalogIndex::build(entries);
        // Viewing "fr" — should NOT show fuzzy even though "es" is fuzzy
        let lens = make_catalog_lens(CatalogKey::new("Hello"), Some("fr"));
        let cmd = resolve_lens(lens, &index, &[]).command.unwrap();
        assert!(!cmd.title.contains("fuzzy"));
    }

    #[test]
    fn resolve_catalog_command_is_find_references() {
        let entries = vec![make_entry("Hello", "es", false)];
        let index = CatalogIndex::build(entries);
        let lens = make_catalog_lens(CatalogKey::new("Hello"), Some("es"));
        let cmd = resolve_lens(lens, &index, &[]).command.unwrap();
        assert_eq!(cmd.command, "babel-lsp.findReferences");
    }

    // ── resolve: source ───────────────────────────────────────────────────────

    #[test]
    fn resolve_source_plural() {
        let index = CatalogIndex::build(vec![]);
        let calls = vec![make_call("Hello", 0), make_call("Hello", 5)];
        let lens = make_source_lens(CatalogKey::new("Hello"));
        let cmd = resolve_lens(lens, &index, &calls).command.unwrap();
        assert_eq!(cmd.title, "used 2 times");
    }

    #[test]
    fn resolve_source_singular() {
        let index = CatalogIndex::build(vec![]);
        let calls = vec![make_call("Hello", 0)];
        let lens = make_source_lens(CatalogKey::new("Hello"));
        let cmd = resolve_lens(lens, &index, &calls).command.unwrap();
        assert_eq!(cmd.title, "used 1 time");
    }

    #[test]
    fn resolve_source_zero_uses() {
        let index = CatalogIndex::build(vec![]);
        let lens = make_source_lens(CatalogKey::new("Hello"));
        let cmd = resolve_lens(lens, &index, &[]).command.unwrap();
        assert_eq!(cmd.title, "used 0 times");
    }

    #[test]
    fn resolve_source_command_is_find_references() {
        let index = CatalogIndex::build(vec![]);
        let lens = make_source_lens(CatalogKey::new("Hello"));
        let cmd = resolve_lens(lens, &index, &[]).command.unwrap();
        assert_eq!(cmd.command, "babel-lsp.findReferences");
    }

    #[test]
    fn resolve_passthrough_when_no_data() {
        let index = CatalogIndex::build(vec![]);
        let lens = CodeLens {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 0 },
            },
            command: None,
            data: None,
        };
        let resolved = resolve_lens(lens, &index, &[]);
        assert!(resolved.command.is_none());
    }
}
