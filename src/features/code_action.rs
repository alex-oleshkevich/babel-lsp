use std::collections::HashMap;

use tower_lsp_server::ls_types::{
    CodeAction, CodeActionKind, CodeActionParams, Command, NumberOrString, Position, Range,
    TextEdit, Uri, WorkspaceEdit,
};

use crate::catalog::index::CatalogEntry;
use crate::util::plural::parse_nplurals;
use crate::util::po_edit::{
    escape_po, flags_line_range, msgstr_replace_range, parse_entry_spans, span_at_line,
    PoEntrySpan,
};

/// Compute code actions for a `.po` / `.pot` catalog buffer.
///
/// `entries` must be all entries for this file (including the header entry so
/// `nplurals` can be extracted from `Plural-Forms`).
pub fn code_actions_for_po(
    params: &CodeActionParams,
    content: &str,
    entries: &[&CatalogEntry],
    uri: &Uri,
) -> Vec<CodeAction> {
    let lines: Vec<&str> = content.lines().collect();
    let spans = parse_entry_spans(content);
    let cursor_line = params.range.start.line;
    let nplurals = nplurals_from_entries(entries);
    let mut actions = Vec::new();

    // ── Single-entry actions ──────────────────────────────────────────────────
    if let Some(span) = span_at_line(&spans, cursor_line) {
        if let Some(entry) = entry_for_span(entries, span) {
            // REQ-ACT-04: copy msgid to all-empty msgstr.
            if !entry.msgid.is_empty() && entry.msgstr.iter().all(|s| s.is_empty()) {
                actions.push(action_copy_msgid(span, entry, &lines, uri, nplurals));
            }

            // REQ-ACT-05: fuzzy toggle.
            if entry.flags.fuzzy {
                if let Some(a) = action_remove_fuzzy(span, &lines, uri) {
                    actions.push(a);
                }
            } else if !entry.flags.obsolete
                && !entry.msgid.is_empty()
                && entry.msgstr.iter().any(|s| !s.is_empty())
            {
                actions.push(action_mark_fuzzy(span, &lines, uri));
            }

            // REQ-ACT-06: fix placeholder mismatch (fires when diagnostic is present).
            let has_mismatch = params.context.diagnostics.iter().any(|d| {
                matches!(&d.code, Some(NumberOrString::String(s)) if s == "po/format-mismatch")
            });
            if has_mismatch && !entry.msgid.is_empty() {
                actions.push(action_fix_placeholder(span, entry, &lines, uri));
            }

            // REQ-ACT-07: add missing plural forms.
            if entry.msgid_plural.is_some() {
                if let Some(n) = nplurals {
                    let existing = span.msgstr_count as u32;
                    if existing < n {
                        actions.push(action_add_plural_forms(span, existing, n, uri));
                    }
                }
            }
        }
    }

    // ── REQ-ACT-09: batch copy across multi-entry selection ───────────────────
    if let Some(batch) = action_batch_copy(&spans, entries, &lines, uri, params.range) {
        actions.push(batch);
    }

    actions
}

/// Compute deterministic fix `TextEdit`s for a PO file given a set of findings.
///
/// `finding_pairs` is `(0-based line, code)` per finding to fix. Only the four
/// deterministic codes are handled: `po/missing-translation`, `po/fuzzy`,
/// `po/format-mismatch`, `po/plural-count`. All others are silently skipped.
///
/// The returned edits are not yet sorted — pass them to `apply_text_edits`.
pub fn fix_edits_for_file(
    content: &str,
    entries: &[&CatalogEntry],
    finding_pairs: &[(u32, &str)],
) -> Vec<TextEdit> {
    let lines: Vec<&str> = content.lines().collect();
    let spans = parse_entry_spans(content);
    let nplurals = nplurals_from_entries(entries);
    let mut edits = Vec::new();

    for &(zero_based_line, code) in finding_pairs {
        let Some(span) = span_at_line(&spans, zero_based_line) else { continue };
        match code {
            "po/missing-translation" => {
                let Some(entry) = entry_for_span(entries, span) else { continue };
                if !entry.msgid.is_empty() && entry.msgstr.iter().all(|s| s.is_empty()) {
                    edits.push(compute_copy_msgid_edit(span, entry, &lines, nplurals));
                }
            }
            "po/fuzzy" => {
                if let Some(edit) = compute_remove_fuzzy_edit(span, &lines) {
                    edits.push(edit);
                }
            }
            "po/format-mismatch" => {
                let Some(entry) = entry_for_span(entries, span) else { continue };
                if !entry.msgid.is_empty() {
                    edits.push(compute_fix_placeholder_edit(span, entry, &lines));
                }
            }
            "po/plural-count" => {
                if let Some(n) = nplurals {
                    let existing = span.msgstr_count as u32;
                    if existing < n {
                        edits.push(compute_add_plural_forms_edit(span, existing, n));
                    }
                }
            }
            _ => {}
        }
    }

    edits
}

// ── Action builders ───────────────────────────────────────────────────────────

fn action_copy_msgid(
    span: &PoEntrySpan,
    entry: &CatalogEntry,
    lines: &[&str],
    uri: &Uri,
    nplurals: Option<u32>,
) -> CodeAction {
    make_quickfix("Copy msgid to msgstr", uri, compute_copy_msgid_edit(span, entry, lines, nplurals))
}

fn compute_copy_msgid_edit(
    span: &PoEntrySpan,
    entry: &CatalogEntry,
    lines: &[&str],
    nplurals: Option<u32>,
) -> TextEdit {
    let range = msgstr_replace_range(span, lines);
    let new_text = if entry.msgid_plural.is_some() {
        let forms = nplurals.unwrap_or(span.msgstr_count as u32).max(1);
        let id_esc = escape_po(&entry.msgid);
        let pl_esc = escape_po(entry.msgid_plural.as_deref().unwrap_or(&entry.msgid));
        (0..forms)
            .map(|i| format!("msgstr[{i}] \"{}\"", if i == 0 { &id_esc } else { &pl_esc }))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        format!("msgstr \"{}\"", escape_po(&entry.msgid))
    };
    TextEdit { range, new_text }
}

fn action_remove_fuzzy(span: &PoEntrySpan, lines: &[&str], uri: &Uri) -> Option<CodeAction> {
    compute_remove_fuzzy_edit(span, lines).map(|edit| make_quickfix("Remove fuzzy", uri, edit))
}

fn compute_remove_fuzzy_edit(span: &PoEntrySpan, lines: &[&str]) -> Option<TextEdit> {
    let fl = span.flags_line?;
    let flags_content = lines.get(fl as usize)?;
    let after = flags_content.strip_prefix("#,")?.trim();
    let remaining: Vec<&str> =
        after.split(',').map(str::trim).filter(|f| *f != "fuzzy").collect();

    let (range, new_text) = if remaining.is_empty() {
        let range = Range {
            start: Position { line: fl, character: 0 },
            end: Position { line: fl + 1, character: 0 },
        };
        (range, String::new())
    } else {
        let range = flags_line_range(span, lines)?;
        (range, format!("#, {}", remaining.join(", ")))
    };
    Some(TextEdit { range, new_text })
}

fn action_mark_fuzzy(span: &PoEntrySpan, lines: &[&str], uri: &Uri) -> CodeAction {
    let (range, new_text) = if let Some(fl) = span.flags_line {
        let flags_content = lines.get(fl as usize).copied().unwrap_or("");
        let after = flags_content.strip_prefix("#,").unwrap_or("").trim();
        let new = if after.is_empty() {
            "#, fuzzy".to_string()
        } else {
            format!("#, fuzzy, {after}")
        };
        let range = flags_line_range(span, lines).unwrap_or_default();
        (range, new)
    } else {
        let range = Range {
            start: Position { line: span.msgid_line, character: 0 },
            end: Position { line: span.msgid_line, character: 0 },
        };
        (range, "#, fuzzy\n".to_string())
    };
    make_quickfix("Mark as fuzzy", uri, TextEdit { range, new_text })
}

fn action_fix_placeholder(
    span: &PoEntrySpan,
    entry: &CatalogEntry,
    lines: &[&str],
    uri: &Uri,
) -> CodeAction {
    make_quickfix(
        "Fix placeholder mismatch: copy msgid to msgstr",
        uri,
        compute_fix_placeholder_edit(span, entry, lines),
    )
}

fn compute_fix_placeholder_edit(
    span: &PoEntrySpan,
    entry: &CatalogEntry,
    lines: &[&str],
) -> TextEdit {
    let range = msgstr_replace_range(span, lines);
    let new_text = format!("msgstr \"{}\"", escape_po(&entry.msgid));
    TextEdit { range, new_text }
}

fn action_add_plural_forms(
    span: &PoEntrySpan,
    existing: u32,
    nplurals: u32,
    uri: &Uri,
) -> CodeAction {
    make_quickfix(
        &format!("Add {} missing plural form(s)", nplurals - existing),
        uri,
        compute_add_plural_forms_edit(span, existing, nplurals),
    )
}

fn compute_add_plural_forms_edit(span: &PoEntrySpan, existing: u32, nplurals: u32) -> TextEdit {
    let insert_line = span.msgstr_end_line + 1;
    let range = Range {
        start: Position { line: insert_line, character: 0 },
        end: Position { line: insert_line, character: 0 },
    };
    let new_text = (existing..nplurals).map(|i| format!("msgstr[{i}] \"\"\n")).collect();
    TextEdit { range, new_text }
}

fn action_batch_copy(
    spans: &[PoEntrySpan],
    entries: &[&CatalogEntry],
    lines: &[&str],
    uri: &Uri,
    selection: Range,
) -> Option<CodeAction> {
    let targets: Vec<(&PoEntrySpan, &CatalogEntry)> = spans
        .iter()
        .filter(|s| s.msgid_line >= selection.start.line && s.msgid_line <= selection.end.line)
        .filter_map(|s| {
            let e = entry_for_span(entries, s)?;
            if !e.msgid.is_empty() && e.msgstr.iter().all(|v| v.is_empty()) {
                Some((s, e))
            } else {
                None
            }
        })
        .collect();

    if targets.len() < 2 {
        return None;
    }

    let edits: Vec<TextEdit> = targets
        .iter()
        .map(|(s, e)| {
            let range = msgstr_replace_range(s, lines);
            let new_text = format!("msgstr \"{}\"", escape_po(&e.msgid));
            TextEdit { range, new_text }
        })
        .collect();

    let n = edits.len();
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    Some(CodeAction {
        title: format!("Copy msgid to all empty msgstr ({n} entries)"),
        kind: Some(CodeActionKind::SOURCE),
        edit: Some(WorkspaceEdit { changes: Some(changes), ..WorkspaceEdit::default() }),
        ..CodeAction::default()
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_quickfix(title: &str, uri: &Uri, edit: TextEdit) -> CodeAction {
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), vec![edit]);
    CodeAction {
        title: title.to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        edit: Some(WorkspaceEdit { changes: Some(changes), ..WorkspaceEdit::default() }),
        ..CodeAction::default()
    }
}

fn entry_for_span<'a>(entries: &[&'a CatalogEntry], span: &PoEntrySpan) -> Option<&'a CatalogEntry> {
    entries.iter().find(|e| e.line > 0 && e.line - 1 == span.msgid_line).copied()
}

fn nplurals_from_entries(entries: &[&CatalogEntry]) -> Option<u32> {
    let header = entries.iter().find(|e| e.msgid.is_empty())?;
    let msgstr = header.msgstr.first()?;
    parse_nplurals(msgstr)
}

/// Command actions for `.po`/`.pot` files: "Update from template" + "Compile catalog" (REQ-CMD-03).
///
/// Returns empty when no locale dirs are configured — the commands have nowhere to operate.
pub fn command_actions_for_po(has_locale_dirs: bool) -> Vec<CodeAction> {
    if !has_locale_dirs {
        return vec![];
    }
    vec![
        make_command_action("Update from template", "babel-lsp.update"),
        make_command_action("Compile catalog", "babel-lsp.compile"),
    ]
}

/// Command action for `babel.cfg` / `pyproject.toml`: "Extract messages" (REQ-CMD-03).
///
/// Returns empty when no locale dirs are configured.
pub fn command_actions_for_config(has_locale_dirs: bool) -> Vec<CodeAction> {
    if !has_locale_dirs {
        return vec![];
    }
    vec![make_command_action("Extract messages", "babel-lsp.extract")]
}

fn make_command_action(title: &str, command_id: &str) -> CodeAction {
    CodeAction {
        title: title.to_string(),
        kind: Some(CodeActionKind::SOURCE),
        command: Some(Command {
            title: title.to_string(),
            command: command_id.to_string(),
            arguments: None,
        }),
        ..CodeAction::default()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tower_lsp_server::ls_types::{CodeActionContext, Diagnostic, TextDocumentIdentifier};

    use super::*;
    use crate::catalog::index::{CatalogEntry, EntryFlags};

    fn uri() -> Uri {
        Uri::from_file_path("/locale/de/messages.po").unwrap()
    }

    fn params(line: u32) -> CodeActionParams {
        params_range(line, line)
    }

    fn params_range(start: u32, end: u32) -> CodeActionParams {
        CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri() },
            range: Range {
                start: Position { line: start, character: 0 },
                end: Position { line: end, character: 0 },
            },
            context: CodeActionContext { diagnostics: vec![], only: None, trigger_kind: None },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        }
    }

    fn params_with_diag(line: u32, code: &str) -> CodeActionParams {
        let mut p = params(line);
        p.context.diagnostics = vec![Diagnostic {
            range: Range::default(),
            code: Some(NumberOrString::String(code.to_string())),
            ..Diagnostic::default()
        }];
        p
    }

    fn entry(msgid: &str, msgstr: &str, line: u32) -> CatalogEntry {
        CatalogEntry {
            locale: "de".into(),
            domain: "messages".into(),
            msgid: msgid.into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec![msgstr.into()],
            flags: EntryFlags { fuzzy: false, obsolete: false },
            file_path: PathBuf::from("/locale/de/messages.po"),
            line,
        }
    }

    fn fuzzy_entry(msgid: &str, msgstr: &str, line: u32) -> CatalogEntry {
        let mut e = entry(msgid, msgstr, line);
        e.flags.fuzzy = true;
        e
    }

    fn plural_entry(msgid: &str, msgid_plural: &str, line: u32) -> CatalogEntry {
        let mut e = entry(msgid, "", line);
        e.msgid_plural = Some(msgid_plural.into());
        e.msgstr = vec!["".into(), "".into()];
        e
    }

    fn header_entry(nplurals: u32) -> CatalogEntry {
        // line=0 is below the 1-based threshold so entry_for_span never matches it.
        let mut e = entry("", "", 0);
        e.msgstr = vec![format!("Plural-Forms: nplurals={nplurals}; plural=(n != 1);\n")];
        e
    }

    fn first_edit(actions: &[CodeAction]) -> Option<&TextEdit> {
        actions.first()?.edit.as_ref()?.changes.as_ref()?.values().next()?.first()
    }

    // ── REQ-ACT-04: copy msgid to msgstr ─────────────────────────────────────

    #[test]
    fn req_act_04_copy_msgid_singular() {
        let content = "msgid \"Save\"\nmsgstr \"\"\n";
        let e = entry("Save", "", 1);
        let actions = code_actions_for_po(&params(0), content, &[&e], &uri());
        let action = actions.iter().find(|a| a.title.contains("Copy msgid")).unwrap();
        let edit = first_edit(std::slice::from_ref(action)).unwrap();
        assert_eq!(edit.new_text, "msgstr \"Save\"");
    }

    #[test]
    fn req_act_04_not_offered_when_msgstr_non_empty() {
        let content = "msgid \"Save\"\nmsgstr \"Speichern\"\n";
        let e = entry("Save", "Speichern", 1);
        let actions = code_actions_for_po(&params(0), content, &[&e], &uri());
        assert!(!actions.iter().any(|a| a.title.contains("Copy msgid")));
    }

    #[test]
    fn req_act_04_copy_msgid_plural() {
        let content =
            "msgid \"%(n)d item\"\nmsgid_plural \"%(n)d items\"\nmsgstr[0] \"\"\nmsgstr[1] \"\"\n";
        let h = header_entry(2);
        let e = plural_entry("%(n)d item", "%(n)d items", 1);
        let actions = code_actions_for_po(&params(0), content, &[&h, &e], &uri());
        let action = actions.iter().find(|a| a.title.contains("Copy msgid")).unwrap();
        let edit = first_edit(std::slice::from_ref(action)).unwrap();
        assert!(edit.new_text.contains("msgstr[0]"));
        assert!(edit.new_text.contains("msgstr[1]"));
        assert!(edit.new_text.contains("%(n)d item"));
        assert!(edit.new_text.contains("%(n)d items"));
    }

    #[test]
    fn req_act_04_escapes_quotes_in_msgid() {
        let content = "msgid \"Say \\\"hello\\\"\"\nmsgstr \"\"\n";
        let e = entry("Say \"hello\"", "", 1);
        let actions = code_actions_for_po(&params(0), content, &[&e], &uri());
        let action = actions.iter().find(|a| a.title.contains("Copy msgid")).unwrap();
        let edit = first_edit(std::slice::from_ref(action)).unwrap();
        assert!(edit.new_text.contains("\\\"hello\\\""), "got: {}", edit.new_text);
    }

    // ── REQ-ACT-05: remove fuzzy ──────────────────────────────────────────────

    #[test]
    fn req_act_05_remove_fuzzy_only_flag_deletes_line() {
        let content = "#, fuzzy\nmsgid \"Save\"\nmsgstr \"Speichern\"\n";
        let e = fuzzy_entry("Save", "Speichern", 2);
        let actions = code_actions_for_po(&params(1), content, &[&e], &uri());
        let action = actions.iter().find(|a| a.title == "Remove fuzzy").unwrap();
        let edit = first_edit(std::slice::from_ref(action)).unwrap();
        assert_eq!(edit.range.start.line, 0);
        assert_eq!(edit.range.end.line, 1);
        assert_eq!(edit.range.end.character, 0);
        assert!(edit.new_text.is_empty());
    }

    #[test]
    fn req_act_05_remove_fuzzy_leaves_other_flags() {
        let content = "#, fuzzy, python-format\nmsgid \"%(n)s\"\nmsgstr \"%(n)s\"\n";
        let e = fuzzy_entry("%(n)s", "%(n)s", 2);
        let actions = code_actions_for_po(&params(1), content, &[&e], &uri());
        let action = actions.iter().find(|a| a.title == "Remove fuzzy").unwrap();
        let edit = first_edit(std::slice::from_ref(action)).unwrap();
        assert_eq!(edit.new_text, "#, python-format");
    }

    // ── REQ-ACT-05: mark as fuzzy ────────────────────────────────────────────

    #[test]
    fn req_act_05_mark_fuzzy_inserts_flags_line() {
        let content = "msgid \"Save\"\nmsgstr \"Speichern\"\n";
        let e = entry("Save", "Speichern", 1);
        let actions = code_actions_for_po(&params(0), content, &[&e], &uri());
        let action = actions.iter().find(|a| a.title == "Mark as fuzzy").unwrap();
        let edit = first_edit(std::slice::from_ref(action)).unwrap();
        assert_eq!(edit.range.start.line, 0);
        assert_eq!(edit.range.end.line, 0);
        assert_eq!(edit.range.start.character, 0);
        assert_eq!(edit.range.end.character, 0);
        assert_eq!(edit.new_text, "#, fuzzy\n");
    }

    #[test]
    fn req_act_05_mark_fuzzy_prepends_to_existing_flags() {
        let content = "#, python-format\nmsgid \"%(n)s\"\nmsgstr \"%(n)s\"\n";
        let e = entry("%(n)s", "%(n)s", 2);
        let actions = code_actions_for_po(&params(1), content, &[&e], &uri());
        let action = actions.iter().find(|a| a.title == "Mark as fuzzy").unwrap();
        let edit = first_edit(std::slice::from_ref(action)).unwrap();
        assert_eq!(edit.new_text, "#, fuzzy, python-format");
    }

    #[test]
    fn req_act_05_mark_fuzzy_not_offered_for_empty_msgstr() {
        let content = "msgid \"Save\"\nmsgstr \"\"\n";
        let e = entry("Save", "", 1);
        let actions = code_actions_for_po(&params(0), content, &[&e], &uri());
        assert!(!actions.iter().any(|a| a.title == "Mark as fuzzy"));
    }

    // ── REQ-ACT-06: fix placeholder mismatch ─────────────────────────────────

    #[test]
    fn req_act_06_fix_placeholder_offered_with_diagnostic() {
        let content = "msgid \"%(n)s item\"\nmsgstr \"%(n)d item\"\n";
        let e = entry("%(n)s item", "%(n)d item", 1);
        let p = params_with_diag(0, "po/format-mismatch");
        let actions = code_actions_for_po(&p, content, &[&e], &uri());
        let action = actions.iter().find(|a| a.title.contains("placeholder")).unwrap();
        let edit = first_edit(std::slice::from_ref(action)).unwrap();
        assert!(edit.new_text.contains("%(n)s item"));
    }

    #[test]
    fn req_act_06_not_offered_without_diagnostic() {
        let content = "msgid \"%(n)s item\"\nmsgstr \"%(n)d item\"\n";
        let e = entry("%(n)s item", "%(n)d item", 1);
        let actions = code_actions_for_po(&params(0), content, &[&e], &uri());
        assert!(!actions.iter().any(|a| a.title.contains("placeholder")));
    }

    // ── REQ-ACT-07: add missing plural forms ─────────────────────────────────

    #[test]
    fn req_act_07_add_missing_plural_forms() {
        let content =
            "msgid \"%(n)d item\"\nmsgid_plural \"%(n)d items\"\nmsgstr[0] \"\"\n";
        let h = header_entry(2);
        let mut e = plural_entry("%(n)d item", "%(n)d items", 1);
        e.msgstr = vec!["".into()];
        let actions = code_actions_for_po(&params(0), content, &[&h, &e], &uri());
        let action = actions.iter().find(|a| a.title.contains("missing plural")).unwrap();
        let edit = first_edit(std::slice::from_ref(action)).unwrap();
        assert!(edit.new_text.contains("msgstr[1]"), "got: {}", edit.new_text);
    }

    #[test]
    fn req_act_07_not_offered_when_forms_complete() {
        let content =
            "msgid \"%(n)d item\"\nmsgid_plural \"%(n)d items\"\nmsgstr[0] \"\"\nmsgstr[1] \"\"\n";
        let h = header_entry(2);
        let e = plural_entry("%(n)d item", "%(n)d items", 1);
        let actions = code_actions_for_po(&params(0), content, &[&h, &e], &uri());
        assert!(!actions.iter().any(|a| a.title.contains("missing plural")));
    }

    // ── REQ-ACT-09: batch copy ────────────────────────────────────────────────

    #[test]
    fn req_act_09_batch_copy_for_multi_entry_selection() {
        let content =
            "msgid \"A\"\nmsgstr \"\"\n\nmsgid \"B\"\nmsgstr \"\"\n\nmsgid \"C\"\nmsgstr \"\"\n";
        let e1 = entry("A", "", 1);
        let e2 = entry("B", "", 4);
        let e3 = entry("C", "", 7);
        let p = params_range(0, 7);
        let actions = code_actions_for_po(&p, content, &[&e1, &e2, &e3], &uri());
        let action = actions
            .iter()
            .find(|a| a.title.contains("entries"))
            .unwrap();
        assert_eq!(action.kind, Some(CodeActionKind::SOURCE));
        let edits = action
            .edit
            .as_ref()
            .unwrap()
            .changes
            .as_ref()
            .unwrap()
            .values()
            .next()
            .unwrap();
        assert_eq!(edits.len(), 3);
    }

    #[test]
    fn req_act_09_not_offered_for_cursor_only() {
        let content = "msgid \"A\"\nmsgstr \"\"\n\nmsgid \"B\"\nmsgstr \"\"\n";
        let e1 = entry("A", "", 1);
        let e2 = entry("B", "", 4);
        let actions = code_actions_for_po(&params(0), content, &[&e1, &e2], &uri());
        assert!(!actions.iter().any(|a| a.title.contains("entries")));
    }

    // ── REQ-CMD-02 / REQ-CMD-03: command actions ─────────────────────────────

    #[test]
    fn req_cmd_02_command_action_carries_command_not_edit() {
        let actions = command_actions_for_po(true);
        for action in &actions {
            assert!(action.command.is_some(), "action {:?} must have a Command", action.title);
            assert!(action.edit.is_none(), "action {:?} must not have an edit", action.title);
        }
    }

    #[test]
    fn req_cmd_03_po_actions_offer_update_and_compile() {
        let actions = command_actions_for_po(true);
        assert!(actions.iter().any(|a| a.title == "Update from template"));
        assert!(actions.iter().any(|a| a.title == "Compile catalog"));
        assert!(!actions.iter().any(|a| a.title == "Extract messages"));
    }

    #[test]
    fn req_cmd_03_config_action_offers_extract() {
        let actions = command_actions_for_config(true);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Extract messages");
    }

    #[test]
    fn req_cmd_03_command_ids_are_correct() {
        let po_actions = command_actions_for_po(true);
        let update = po_actions.iter().find(|a| a.title == "Update from template").unwrap();
        assert_eq!(update.command.as_ref().unwrap().command, "babel-lsp.update");
        let compile = po_actions.iter().find(|a| a.title == "Compile catalog").unwrap();
        assert_eq!(compile.command.as_ref().unwrap().command, "babel-lsp.compile");

        let cfg_actions = command_actions_for_config(true);
        assert_eq!(cfg_actions[0].command.as_ref().unwrap().command, "babel-lsp.extract");
    }

    #[test]
    fn req_cmd_03_no_actions_when_no_locale_dirs() {
        assert!(command_actions_for_po(false).is_empty());
        assert!(command_actions_for_config(false).is_empty());
    }

    #[test]
    fn req_cmd_03_command_actions_are_source_kind() {
        for action in command_actions_for_po(true).iter().chain(command_actions_for_config(true).iter()) {
            assert_eq!(action.kind, Some(CodeActionKind::SOURCE), "action '{}' must be SOURCE kind", action.title);
        }
    }
}
