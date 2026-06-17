use std::collections::HashMap;

use tower_lsp_server::ls_types::{
    CodeAction, CodeActionKind, Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range,
    TextEdit, Uri, WorkspaceEdit,
};
use tree_sitter::{Node, Parser};

use crate::catalog::index::{CatalogIndex, CatalogKey};
use crate::extract::types::TranslationFunc;
use crate::util::po_edit::escape_po;

const MIN_CHAR_LEN: usize = 3;

const UI_CALL_NAMES: &[&str] = &[
    "success",
    "error",
    "warning",
    "info",
    "message",
    "flash",
    "notify",
    "alert",
    "add_message",
    "add_error",
    "add_warning",
    "add_success",
];

const LOG_METHOD_NAMES: &[&str] = &[
    "debug",
    "info",
    "warning",
    "warn",
    "error",
    "critical",
    "exception",
    "log",
];

const LOG_OBJECT_NAMES: &[&str] = &["logger", "logging", "log", "app", "current_app"];

/// Detect hardcoded string literals in a source file that appear to be
/// user-facing prose not wrapped in a translation call.
///
/// Dispatches to the appropriate language-specific detector based on file
/// extension. Returns an empty slice for non-Python files (Jinja detection
/// is deferred).
///
/// This function is called only when `config.detect_hardcoded_strings` is true.
pub fn check_source(
    source: &[u8],
    uri: &Uri,
    extra_keywords: &HashMap<String, TranslationFunc>,
) -> Vec<Diagnostic> {
    let ext = uri
        .to_file_path()
        .and_then(|p| p.extension().and_then(|e| e.to_str()).map(str::to_owned))
        .unwrap_or_default();
    match ext.as_str() {
        "py" => check_python(source, extra_keywords),
        _ => vec![],
    }
}

/// Detect hardcoded string literals in Python source (REQ-HARD-01..05).
///
/// Returns `msg/hardcoded-string` diagnostics at Information severity for any
/// string literal in a user-facing position that reads like prose and survives
/// the exclusion list.
pub fn check_python(
    source: &[u8],
    extra_keywords: &HashMap<String, TranslationFunc>,
) -> Vec<Diagnostic> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .expect("tree-sitter-python load");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    let mut diags = Vec::new();
    walk(tree.root_node(), source, extra_keywords, &mut diags);
    diags
}

// ── AST walk ──────────────────────────────────────────────────────────────────

fn walk(
    node: Node,
    source: &[u8],
    extra: &HashMap<String, TranslationFunc>,
    out: &mut Vec<Diagnostic>,
) {
    match node.kind() {
        "return_statement" => handle_return(node, source, extra, out),
        "raise_statement" => handle_raise(node, source, extra, out),
        "call" => handle_call(node, source, extra, out),
        "assignment" => handle_assignment(node, source, extra, out),
        _ => recurse(node, source, extra, out),
    }
}

fn recurse(
    node: Node,
    source: &[u8],
    extra: &HashMap<String, TranslationFunc>,
    out: &mut Vec<Diagnostic>,
) {
    let mut cursor = node.walk();
    let children: Vec<_> = node.named_children(&mut cursor).collect();
    for child in children {
        walk(child, source, extra, out);
    }
}

/// Check a `return_statement` node.
///
/// Only direct string literals and call expressions in the return value are
/// inspected — binary operators, ternaries, etc. are skipped to stay conservative.
fn handle_return(
    node: Node,
    source: &[u8],
    extra: &HashMap<String, TranslationFunc>,
    out: &mut Vec<Diagnostic>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "string" => check_string_candidate(child, source, out),
            // Recurse into call expressions — handles _("msg") and UI calls.
            "call" => walk(child, source, extra, out),
            // All other expressions are skipped (conservative: no ternaries, binops, etc.)
            _ => {}
        }
    }
}

/// Check a `raise_statement` node.
///
/// Flags the first string argument of the raised exception, e.g.
/// `raise ValidationError("Email is required")`.
fn handle_raise(
    node: Node,
    source: &[u8],
    extra: &HashMap<String, TranslationFunc>,
    out: &mut Vec<Diagnostic>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "call" {
            continue;
        }
        // Skip `raise _("already wrapped")`.
        if let Some(func_node) = child.child_by_field_name("function") {
            let callee = resolve_callee(func_node, source).unwrap_or_default();
            if TranslationFunc::resolve(&callee, extra).is_some() {
                continue;
            }
        }
        if let Some(args) = child.child_by_field_name("arguments") {
            let mut arg_cursor = args.walk();
            if let Some(first) = args.named_children(&mut arg_cursor).next() {
                match first.kind() {
                    "string" => check_string_candidate(first, source, out),
                    "keyword_argument" => {
                        if let Some(val) = first.child_by_field_name("value") {
                            if val.kind() == "string" {
                                check_string_candidate(val, source, out);
                            }
                        }
                    }
                    _ => {} // non-string first arg — skip
                }
            }
        }
    }
}

/// Check a `call` node.
///
/// - Translation calls: skips the entire subtree (strings already wrapped).
/// - Log calls: skips so their string args are never flagged.
/// - UI-rendering calls: flags any string arguments.
/// - Other calls: recurses normally.
fn handle_call(
    node: Node,
    source: &[u8],
    extra: &HashMap<String, TranslationFunc>,
    out: &mut Vec<Diagnostic>,
) {
    let Some(func_node) = node.child_by_field_name("function") else {
        recurse(node, source, extra, out);
        return;
    };

    let callee = resolve_callee(func_node, source).unwrap_or_default();

    if TranslationFunc::resolve(&callee, extra).is_some() {
        return; // already wrapped
    }

    if is_log_call(func_node, source) {
        return; // log messages are not user-facing (REQ-HARD-05)
    }

    if is_ui_call(&callee) {
        if let Some(args) = node.child_by_field_name("arguments") {
            let mut cursor = args.walk();
            for arg in args.named_children(&mut cursor) {
                match arg.kind() {
                    "string" => check_string_candidate(arg, source, out),
                    "keyword_argument" => {
                        if let Some(val) = arg.child_by_field_name("value") {
                            if val.kind() == "string" {
                                check_string_candidate(val, source, out);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        return;
    }

    recurse(node, source, extra, out);
}

/// Check an `assignment` node.
///
/// Skips dunder attributes (`__version__`) and `UPPER_CASE` module constants —
/// these are metadata, not user-facing strings (REQ-HARD-05).
fn handle_assignment(
    node: Node,
    source: &[u8],
    extra: &HashMap<String, TranslationFunc>,
    out: &mut Vec<Diagnostic>,
) {
    if let Some(lhs) = node.child_by_field_name("left") {
        let lhs_text = lhs.utf8_text(source).unwrap_or_default();
        if is_dunder_or_constant(lhs_text) {
            return;
        }
    }
    recurse(node, source, extra, out);
}

// ── Candidate checking ────────────────────────────────────────────────────────

fn check_string_candidate(node: Node, source: &[u8], out: &mut Vec<Diagnostic>) {
    let raw = node.utf8_text(source).unwrap_or_default();
    let Some(content) = extract_string_content(raw) else {
        return; // f-string, bytes, or unrecognised prefix
    };
    if !is_prose(&content) {
        return;
    }
    if is_excluded(&content) {
        return;
    }
    out.push(make_diag(node_range(node), &content));
}

// ── Heuristics (REQ-HARD-04, REQ-HARD-05) ────────────────────────────────────

/// True if the content looks like prose meant for a person:
/// - contains at least one space or newline, **or**
/// - is a single capitalized word (first character is uppercase).
///
/// Single words without spaces (including PascalCase identifiers) are never
/// flagged — they are likely class names or enum values, not user-facing prose.
fn is_prose(content: &str) -> bool {
    if content.is_empty() {
        return false;
    }
    if !content.contains(' ') && !content.contains('\n') {
        return false;
    }
    if content.contains(' ') {
        return true;
    }
    content
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
}

/// True if the content should be excluded from detection even though it reads
/// like prose (REQ-HARD-05).
fn is_excluded(content: &str) -> bool {
    // Too short — too ambiguous
    if content.chars().count() < MIN_CHAR_LEN {
        return true;
    }
    // URL
    if content.starts_with("http://") || content.starts_with("https://") {
        return true;
    }
    // Unix path
    if content.starts_with('/') || content.starts_with("./") || content.starts_with("../") {
        return true;
    }
    // Email-like (no spaces, contains @)
    if !content.contains(' ') && content.contains('@') {
        return true;
    }
    false
}

fn is_dunder_or_constant(name: &str) -> bool {
    if name.starts_with("__") && name.ends_with("__") {
        return true;
    }
    let chars: Vec<char> = name.chars().collect();
    !chars.is_empty()
        && chars
            .iter()
            .all(|c| c.is_uppercase() || *c == '_' || c.is_ascii_digit())
        && chars.iter().any(|c| c.is_uppercase())
}

fn is_log_call(func_node: Node, source: &[u8]) -> bool {
    if func_node.kind() != "attribute" {
        return false;
    }
    let method = func_node
        .child_by_field_name("attribute")
        .and_then(|n| n.utf8_text(source).ok())
        .unwrap_or_default();
    if !LOG_METHOD_NAMES.contains(&method) {
        return false;
    }
    let obj = func_node
        .child_by_field_name("object")
        .and_then(|n| n.utf8_text(source).ok())
        .unwrap_or_default();
    let obj_base = obj.split('.').next_back().unwrap_or(obj);
    LOG_OBJECT_NAMES
        .iter()
        .any(|&n| obj_base.eq_ignore_ascii_case(n))
}

fn is_ui_call(callee: &str) -> bool {
    UI_CALL_NAMES.contains(&callee)
}

// ── String content extraction ─────────────────────────────────────────────────

/// Strip the prefix, quotes, and raw markers from a Python string literal,
/// returning the unquoted content. Returns `None` for f-strings and byte strings
/// (not valid msgids).
fn extract_string_content(raw: &str) -> Option<String> {
    let mut s = raw;
    loop {
        match s.chars().next()? {
            'r' | 'R' => {
                s = &s[1..];
            }
            'u' | 'U' => {
                s = &s[1..];
            }
            'f' | 'F' | 'b' | 'B' => return None,
            _ => break,
        }
    }
    let open = if s.starts_with("\"\"\"") {
        "\"\"\""
    } else if s.starts_with("'''") {
        "'''"
    } else if s.starts_with('"') {
        "\""
    } else if s.starts_with('\'') {
        "'"
    } else {
        return None;
    };
    let content = s.strip_prefix(open)?.strip_suffix(open)?;
    Some(content.to_string())
}

// ── Callee resolution ─────────────────────────────────────────────────────────

fn resolve_callee(node: Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => node.utf8_text(source).ok().map(str::to_string),
        "attribute" => node
            .child_by_field_name("attribute")
            .and_then(|n| n.utf8_text(source).ok())
            .map(str::to_string),
        _ => None,
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn node_range(node: Node) -> Range {
    let s = node.start_position();
    let e = node.end_position();
    Range {
        start: Position {
            line: s.row as u32,
            character: s.column as u32,
        },
        end: Position {
            line: e.row as u32,
            character: e.column as u32,
        },
    }
}

fn make_diag(range: Range, content: &str) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::INFORMATION),
        code: Some(NumberOrString::String("msg/hardcoded-string".to_string())),
        code_description: None,
        source: Some("babel-lsp".to_string()),
        message: format!(
            "Hardcoded string '{}' — looks translatable. Did you mean to wrap it in _()?",
            content
        ),
        related_information: None,
        tags: None,
        data: None,
    }
}

// ── Extract message quick fix ─────────────────────────────────────────────────

/// Generate "Extract message" code actions for `msg/hardcoded-string` diagnostics.
///
/// `pot` is `None` when no `.pot` template exists (wrap-only variant per REQ-HARD-10).
/// `locale_po_files` enables the "add to all locales" variant (REQ-HARD-07).
pub fn code_actions_for_hardcoded(
    diagnostics: &[Diagnostic],
    source: &str,
    source_uri: &Uri,
    keyword: &str,
    pot: Option<(&Uri, &str)>,
    index: &CatalogIndex,
    locale_po_files: &[(&Uri, &str)],
) -> Vec<CodeAction> {
    let lines: Vec<&str> = source.lines().collect();
    let mut actions = Vec::new();

    for diag in diagnostics {
        if !matches!(&diag.code, Some(NumberOrString::String(s)) if s == "msg/hardcoded-string") {
            continue;
        }

        let range = diag.range;
        if range.start.line != range.end.line {
            continue;
        }

        let Some(line) = lines.get(range.start.line as usize) else {
            continue;
        };

        // Column positions are byte offsets (tree-sitter node_range uses byte columns).
        let start = range.start.character as usize;
        let end = range.end.character as usize;
        if start >= end || end > line.len() {
            continue;
        }
        if !line.is_char_boundary(start) || !line.is_char_boundary(end) {
            continue;
        }

        let raw_literal = &line[start..end];
        let Some(content) = extract_string_content(raw_literal) else {
            continue;
        };

        // Wrap the raw literal to preserve Python escape sequences exactly.
        let source_edit = TextEdit {
            range,
            new_text: format!("{}({})", keyword, raw_literal),
        };

        match pot {
            None => {
                // No .pot template — wrap only (REQ-HARD-10).
                let mut changes = HashMap::new();
                changes.insert(source_uri.clone(), vec![source_edit]);
                actions.push(quickfix(format!("Wrap in {keyword}()"), changes));
            }
            Some((pot_uri, pot_content)) => {
                if index.is_in_pot(&CatalogKey::new(&content)) {
                    // Already in catalog — wrap only, no duplicate .pot entry (REQ-HARD-08).
                    let mut changes = HashMap::new();
                    changes.insert(source_uri.clone(), vec![source_edit]);
                    actions.push(quickfix(
                        format!("Wrap in {keyword}() (message already in catalog)"),
                        changes,
                    ));
                } else {
                    // New msgid — wrap source and append to .pot (REQ-HARD-06).
                    let pot_edit = pot_append_edit(pot_content, &content);

                    let mut changes = HashMap::new();
                    changes.insert(source_uri.clone(), vec![source_edit.clone()]);
                    changes.insert(pot_uri.clone(), vec![pot_edit.clone()]);
                    actions.push(quickfix("Extract message", changes));

                    // "Add to all locales" variant (REQ-HARD-07).
                    if !locale_po_files.is_empty() {
                        let mut changes = HashMap::new();
                        changes.insert(source_uri.clone(), vec![source_edit.clone()]);
                        changes.insert(pot_uri.clone(), vec![pot_edit]);
                        for &(po_uri, po_content) in locale_po_files {
                            changes.insert(
                                po_uri.clone(),
                                vec![pot_append_edit(po_content, &content)],
                            );
                        }
                        actions.push(quickfix("Extract message and add to all locales", changes));
                    }
                }
            }
        }
    }

    actions
}

fn quickfix(title: impl Into<String>, changes: HashMap<Uri, Vec<TextEdit>>) -> CodeAction {
    CodeAction {
        title: title.into(),
        kind: Some(CodeActionKind::QUICKFIX),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..WorkspaceEdit::default()
        }),
        ..CodeAction::default()
    }
}

/// Insert `\nmsgid "<escaped>"\nmsgstr ""\n` at the end of a PO/POT file.
fn pot_append_edit(content: &str, msgid: &str) -> TextEdit {
    let line_count = content.lines().count() as u32;
    let (insert_line, insert_char) = if content.is_empty() || content.ends_with('\n') {
        (line_count, 0)
    } else {
        let last_line_len = content.lines().last().map(str::len).unwrap_or(0) as u32;
        (line_count.saturating_sub(1), last_line_len)
    };
    TextEdit {
        range: Range {
            start: Position {
                line: insert_line,
                character: insert_char,
            },
            end: Position {
                line: insert_line,
                character: insert_char,
            },
        },
        new_text: format!("\nmsgid \"{}\"\nmsgstr \"\"\n", escape_po(msgid)),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(src: &str) -> Vec<Diagnostic> {
        check_python(src.as_bytes(), &HashMap::new())
    }

    fn has(diags: &[Diagnostic]) -> bool {
        diags.iter().any(
            |d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/hardcoded-string"),
        )
    }

    // ── REQ-HARD-02: severity is Information ───────────────────────────────────

    #[test]
    fn req_hard_02_severity_is_information() {
        let diags = detect("def v():\n    return \"Order placed\"");
        let d = diags
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/hardcoded-string"))
            .expect("expected msg/hardcoded-string");
        assert_eq!(d.severity, Some(DiagnosticSeverity::INFORMATION));
    }

    #[test]
    fn req_hard_02_source_is_babel_lsp() {
        let diags = detect("def v():\n    return \"Order placed\"");
        let d = diags
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/hardcoded-string"))
            .expect("expected msg/hardcoded-string");
        assert_eq!(d.source.as_deref(), Some("babel-lsp"));
    }

    // ── REQ-HARD-03: Gate 1 — user-facing positions ────────────────────────────

    #[test]
    fn req_hard_03_return_string_fires() {
        let diags = detect("def v():\n    return \"Order placed\"");
        assert!(has(&diags), "return position should fire");
    }

    #[test]
    fn req_hard_03_ui_call_arg_fires() {
        let diags = detect("messages.success(request, \"Order saved\")");
        assert!(has(&diags), "UI call arg should fire");
    }

    #[test]
    fn req_hard_03_raise_fires() {
        let diags = detect("raise ValidationError(\"Email is required\")");
        assert!(has(&diags), "raise exception message should fire");
    }

    #[test]
    fn req_hard_03_dict_key_silent() {
        let diags = detect("data = {\"Status\": \"Active\"}");
        assert!(!has(&diags), "dict key and value are not user-facing");
    }

    #[test]
    fn req_hard_03_assignment_value_silent() {
        let diags = detect("x = \"Order placed\"");
        assert!(!has(&diags), "plain assignment is not user-facing");
    }

    #[test]
    fn req_hard_03_return_call_not_string_silent() {
        // return of a non-string call — the function result isn't flagged
        let diags = detect("def v():\n    return get_message()");
        assert!(
            !has(&diags),
            "return of a call result is not a string literal"
        );
    }

    // ── REQ-HARD-04: Gate 2 — prose shape ────────────────────────────────────

    #[test]
    fn req_hard_04_has_space_fires() {
        let diags = detect("def v():\n    return \"Order placed\"");
        assert!(has(&diags), "string with space is prose");
    }

    #[test]
    fn req_hard_04_single_capitalized_word_no_space_silent() {
        // Single capitalized words without spaces are identifiers/class names — not prose.
        let diags = detect("def v():\n    return \"Checkout\"");
        assert!(
            !has(&diags),
            "single capitalized word without space is not prose"
        );
    }

    #[test]
    fn req_hard_04_capitalized_multi_word_fires() {
        let diags = detect("def v():\n    return \"Checkout now\"");
        assert!(has(&diags), "capitalized multi-word string is prose");
    }

    #[test]
    fn req_hard_04_lowercase_single_token_silent() {
        let diags = detect("def v():\n    return \"submit\"");
        assert!(!has(&diags), "lowercase single token is not prose");
    }

    #[test]
    fn req_hard_04_single_digit_silent() {
        let diags = detect("def v():\n    return \"42\"");
        assert!(!has(&diags), "number string is not prose");
    }

    // ── REQ-HARD-05: Gate 3 — exclusions ─────────────────────────────────────

    #[test]
    fn req_hard_05_already_wrapped_silent() {
        let diags = detect("def v():\n    return _(\"Checkout\")");
        assert!(!has(&diags), "already wrapped in _() should be silent");
    }

    #[test]
    fn req_hard_05_url_silent() {
        let diags = detect("def v():\n    return \"https://example.com\"");
        assert!(!has(&diags), "URL should be silent");
    }

    #[test]
    fn req_hard_05_unix_path_silent() {
        let diags = detect("def v():\n    return \"/orders\"");
        assert!(!has(&diags), "Unix path should be silent");
    }

    #[test]
    fn req_hard_05_relative_path_silent() {
        let diags = detect("def v():\n    return \"./static/img\"");
        assert!(!has(&diags), "relative path should be silent");
    }

    #[test]
    fn req_hard_05_email_silent() {
        let diags = detect("def v():\n    return \"user@example.com\"");
        assert!(!has(&diags), "email address should be silent");
    }

    #[test]
    fn req_hard_05_log_call_silent() {
        let diags = detect("logger.info(\"cache miss for user\")");
        assert!(!has(&diags), "log call args should be silent");
    }

    #[test]
    fn req_hard_05_log_warning_silent() {
        let diags = detect("log.warning(\"Request failed with status\")");
        assert!(!has(&diags), "log.warning args should be silent");
    }

    #[test]
    fn req_hard_05_dunder_silent() {
        let diags = detect("__version__ = \"First Release\"");
        assert!(!has(&diags), "dunder assignment should be silent");
    }

    #[test]
    fn req_hard_05_upper_constant_silent() {
        let diags = detect("ERROR_MSG = \"Something went wrong\"");
        assert!(
            !has(&diags),
            "UPPER_CASE constant assignment should be silent"
        );
    }

    #[test]
    fn req_hard_05_too_short_silent() {
        let diags = detect("def v():\n    return \"OK\"");
        assert!(
            !has(&diags),
            "short string below min length should be silent"
        );
    }

    // ── Overlap: method names in both UI and LOG lists ───────────────────────

    #[test]
    fn overlap_messages_info_fires_as_ui_call() {
        // "info" is in both UI_CALL_NAMES and LOG_METHOD_NAMES.
        // messages.info is a UI call (object "messages" is not a logger).
        let diags = detect("messages.info(request, \"Order confirmed\")");
        assert!(has(&diags), "messages.info arg should fire as UI call");
    }

    #[test]
    fn overlap_logger_error_silent_as_log_call() {
        // "error" is in both lists. logger.error is a log call (object "logger" is known logger).
        let diags = detect("logger.error(\"DB connection failed\")");
        assert!(
            !has(&diags),
            "logger.error arg should be silent as log call"
        );
    }

    #[test]
    fn overlap_logging_warning_silent() {
        // "warning" is in both lists. logging.warning is a log call.
        let diags = detect("logging.warning(\"Deprecated API called\")");
        assert!(
            !has(&diags),
            "logging.warning arg should be silent as log call"
        );
    }

    // ── Extra behavioural checks ──────────────────────────────────────────────

    #[test]
    fn extra_keywords_exclude_custom_translation_func() {
        let mut extra = HashMap::new();
        extra.insert("t".to_string(), TranslationFunc::Gettext);
        let diags = check_python(b"def v():\n    return t(\"Checkout\")", &extra);
        assert!(
            !has(&diags),
            "custom translation func should be treated as wrapped"
        );
    }

    #[test]
    fn raise_inside_translation_call_silent() {
        // raise _(Error("...")) — contrived but ensures we don't double-flag
        let diags = detect("raise _(SomeError(\"Checkout\"))");
        assert!(
            !has(&diags),
            "raise wrapping a translation call should be silent"
        );
    }

    #[test]
    fn multiple_findings_in_one_file() {
        let src = "def v():\n    return \"Order placed\"\ndef w():\n    return \"Save changes\"";
        let diags = detect(src);
        assert_eq!(diags.len(), 2, "should find two hardcoded strings");
    }

    // ── Extract message quick fix ─────────────────────────────────────────────

    use crate::catalog::index::{CatalogEntry, CatalogIndex, EntryFlags};
    use std::path::PathBuf;

    fn source_uri() -> Uri {
        Uri::from_file_path("/app/views.py").unwrap()
    }

    fn pot_uri() -> Uri {
        Uri::from_file_path("/locale/messages.pot").unwrap()
    }

    fn de_uri() -> Uri {
        Uri::from_file_path("/locale/de/LC_MESSAGES/messages.po").unwrap()
    }

    fn make_diag_at(line: u32, start_char: u32, end_char: u32) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position {
                    line,
                    character: start_char,
                },
                end: Position {
                    line,
                    character: end_char,
                },
            },
            code: Some(NumberOrString::String("msg/hardcoded-string".to_string())),
            ..Diagnostic::default()
        }
    }

    fn empty_index() -> CatalogIndex {
        CatalogIndex::build(vec![])
    }

    fn index_with_pot_entry(msgid: &str) -> CatalogIndex {
        let entry = CatalogEntry {
            locale: String::new(),
            domain: "messages".to_string(),
            msgid: msgid.to_string(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec![String::new()],
            flags: EntryFlags {
                fuzzy: false,
                obsolete: false,
            },
            file_path: PathBuf::from("/locale/messages.pot"),
            line: 1,
        };
        CatalogIndex::build(vec![entry])
    }

    fn first_source_edit(action: &CodeAction) -> Option<&TextEdit> {
        action
            .edit
            .as_ref()?
            .changes
            .as_ref()?
            .get(&source_uri())?
            .first()
    }

    fn first_pot_edit(action: &CodeAction) -> Option<&TextEdit> {
        action
            .edit
            .as_ref()?
            .changes
            .as_ref()?
            .get(&pot_uri())?
            .first()
    }

    // ── REQ-HARD-06: extract wraps source and appends .pot ────────────────────

    #[test]
    fn req_hard_06_extract_wraps_literal_in_source() {
        // `return "Order placed"` — the literal is at columns 11-25 on line 1.
        let source = "def place_order():\n    return \"Order placed\"\n";
        let diag = make_diag_at(1, 11, 25);
        let index = empty_index();
        let pot_content = "msgid \"\"\nmsgstr \"\"\n";
        let actions = code_actions_for_hardcoded(
            &[diag],
            source,
            &source_uri(),
            "_",
            Some((&pot_uri(), pot_content)),
            &index,
            &[],
        );
        let action = actions
            .iter()
            .find(|a| a.title == "Extract message")
            .unwrap();
        let edit = first_source_edit(action).unwrap();
        assert_eq!(edit.new_text, "_(\"Order placed\")");
    }

    #[test]
    fn req_hard_06_extract_appends_pot_entry() {
        let source = "def place_order():\n    return \"Order placed\"\n";
        let diag = make_diag_at(1, 11, 25);
        let index = empty_index();
        let pot_content = "msgid \"\"\nmsgstr \"\"\n";
        let actions = code_actions_for_hardcoded(
            &[diag],
            source,
            &source_uri(),
            "_",
            Some((&pot_uri(), pot_content)),
            &index,
            &[],
        );
        let action = actions
            .iter()
            .find(|a| a.title == "Extract message")
            .unwrap();
        let pot_edit = first_pot_edit(action).unwrap();
        assert!(
            pot_edit.new_text.contains("msgid \"Order placed\""),
            "got: {}",
            pot_edit.new_text
        );
        assert!(
            pot_edit.new_text.contains("msgstr \"\""),
            "got: {}",
            pot_edit.new_text
        );
    }

    #[test]
    fn req_hard_06_extract_is_atomic_workspace_edit() {
        let source = "def place_order():\n    return \"Order placed\"\n";
        let diag = make_diag_at(1, 11, 25);
        let index = empty_index();
        let actions = code_actions_for_hardcoded(
            &[diag],
            source,
            &source_uri(),
            "_",
            Some((&pot_uri(), "msgid \"\"\nmsgstr \"\"\n")),
            &index,
            &[],
        );
        let action = actions
            .iter()
            .find(|a| a.title == "Extract message")
            .unwrap();
        let changes = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
        assert!(changes.contains_key(&source_uri()), "must have source edit");
        assert!(changes.contains_key(&pot_uri()), "must have .pot edit");
    }

    // ── REQ-HARD-07: seed all locales variant ─────────────────────────────────

    #[test]
    fn req_hard_07_locale_seeding_variant_offered_when_po_files_present() {
        let source = "def v():\n    return \"Order placed\"\n";
        let diag = make_diag_at(1, 11, 25);
        let index = empty_index();
        let de_content = "msgid \"\"\nmsgstr \"\"\n";
        let actions = code_actions_for_hardcoded(
            &[diag],
            source,
            &source_uri(),
            "_",
            Some((&pot_uri(), "msgid \"\"\nmsgstr \"\"\n")),
            &index,
            &[(&de_uri(), de_content)],
        );
        assert!(
            actions
                .iter()
                .any(|a| a.title == "Extract message and add to all locales"),
            "locale seeding action should be offered when po files are present"
        );
    }

    #[test]
    fn req_hard_07_locale_seeding_appends_to_po_file() {
        let source = "def v():\n    return \"Order placed\"\n";
        let diag = make_diag_at(1, 11, 25);
        let index = empty_index();
        let de_content = "msgid \"\"\nmsgstr \"\"\n";
        let actions = code_actions_for_hardcoded(
            &[diag],
            source,
            &source_uri(),
            "_",
            Some((&pot_uri(), "msgid \"\"\nmsgstr \"\"\n")),
            &index,
            &[(&de_uri(), de_content)],
        );
        let action = actions
            .iter()
            .find(|a| a.title == "Extract message and add to all locales")
            .unwrap();
        let changes = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
        let po_edits = changes.get(&de_uri()).unwrap();
        assert!(
            po_edits[0].new_text.contains("msgid \"Order placed\""),
            "locale .po should get the new entry"
        );
    }

    #[test]
    fn req_hard_07_locale_seeding_not_offered_without_po_files() {
        let source = "def v():\n    return \"Order placed\"\n";
        let diag = make_diag_at(1, 11, 25);
        let index = empty_index();
        let actions = code_actions_for_hardcoded(
            &[diag],
            source,
            &source_uri(),
            "_",
            Some((&pot_uri(), "msgid \"\"\nmsgstr \"\"\n")),
            &index,
            &[],
        );
        assert!(
            !actions.iter().any(|a| a.title.contains("all locales")),
            "locale seeding action should not appear when no .po files"
        );
    }

    // ── REQ-HARD-08: reuse existing msgid ─────────────────────────────────────

    #[test]
    fn req_hard_08_already_in_catalog_offers_wrap_only() {
        let source = "def v():\n    return \"Checkout\"\n";
        // "Checkout" is 10 chars: columns 11-21
        let diag = make_diag_at(1, 11, 21);
        let index = index_with_pot_entry("Checkout");
        let actions = code_actions_for_hardcoded(
            &[diag],
            source,
            &source_uri(),
            "_",
            Some((&pot_uri(), "msgid \"\"\nmsgstr \"\"\n")),
            &index,
            &[],
        );
        assert!(
            actions
                .iter()
                .any(|a| a.title.contains("already in catalog")),
            "should say 'already in catalog'"
        );
        assert!(
            !actions.iter().any(|a| a.title == "Extract message"),
            "should not offer full extract when already in catalog"
        );
    }

    #[test]
    fn req_hard_08_already_in_catalog_no_pot_edit() {
        let source = "def v():\n    return \"Checkout\"\n";
        let diag = make_diag_at(1, 11, 21);
        let index = index_with_pot_entry("Checkout");
        let actions = code_actions_for_hardcoded(
            &[diag],
            source,
            &source_uri(),
            "_",
            Some((&pot_uri(), "msgid \"\"\nmsgstr \"\"\n")),
            &index,
            &[],
        );
        let action = actions
            .iter()
            .find(|a| a.title.contains("already in catalog"))
            .unwrap();
        let changes = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
        assert!(
            !changes.contains_key(&pot_uri()),
            "no .pot edit for existing msgid"
        );
        assert!(
            changes.contains_key(&source_uri()),
            "source edit still present"
        );
    }

    // ── REQ-HARD-10: correctness gate — no .pot → wrap only ──────────────────

    #[test]
    fn req_hard_10_no_pot_file_offers_wrap_only() {
        let source = "def v():\n    return \"Order placed\"\n";
        let diag = make_diag_at(1, 11, 25);
        let index = empty_index();
        let actions =
            code_actions_for_hardcoded(&[diag], source, &source_uri(), "_", None, &index, &[]);
        assert_eq!(actions.len(), 1);
        assert!(
            actions[0].title.starts_with("Wrap in"),
            "should offer wrap-only action"
        );
        let changes = actions[0].edit.as_ref().unwrap().changes.as_ref().unwrap();
        assert_eq!(changes.len(), 1, "only source edit, no .pot edit");
    }

    #[test]
    fn req_hard_10_diagnostic_ignored_for_non_hardcoded_code() {
        let source = "def v():\n    return \"Order placed\"\n";
        let mut diag = make_diag_at(1, 11, 25);
        diag.code = Some(NumberOrString::String("po/missing-translation".to_string()));
        let index = empty_index();
        let actions = code_actions_for_hardcoded(
            &[diag],
            source,
            &source_uri(),
            "_",
            Some((&pot_uri(), "")),
            &index,
            &[],
        );
        assert!(
            actions.is_empty(),
            "non-hardcoded diagnostic should be ignored"
        );
    }

    // ── pot_append_edit helpers ───────────────────────────────────────────────

    #[test]
    fn pot_append_edit_inserts_at_end_when_trailing_newline() {
        let content = "msgid \"\"\nmsgstr \"\"\n";
        let edit = pot_append_edit(content, "Hello world");
        assert!(
            edit.new_text.starts_with('\n'),
            "should start with blank separator"
        );
        assert!(edit.new_text.contains("msgid \"Hello world\""));
        assert!(edit.new_text.contains("msgstr \"\""));
        // Insertion point is past the last line (line_count=2, char=0)
        assert_eq!(edit.range.start.line, 2);
        assert_eq!(edit.range.start.character, 0);
    }

    #[test]
    fn pot_append_edit_inserts_at_end_when_no_trailing_newline() {
        let content = "msgid \"\"\nmsgstr \"\"";
        let edit = pot_append_edit(content, "Hello world");
        // No trailing newline: last line is "msgstr \"\"" (9 chars)
        assert_eq!(edit.range.start.line, 1);
        assert_eq!(edit.range.start.character, 9);
    }

    #[test]
    fn pot_append_edit_escapes_special_chars() {
        let edit = pot_append_edit("", "Say \"hello\"");
        assert!(
            edit.new_text.contains("msgid \"Say \\\"hello\\\"\""),
            "got: {}",
            edit.new_text
        );
    }
}
