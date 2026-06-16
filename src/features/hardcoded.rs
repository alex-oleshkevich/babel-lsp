use std::collections::HashMap;

use tower_lsp_server::ls_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range, Uri};
use tree_sitter::{Node, Parser};

use crate::extract::types::TranslationFunc;

const MIN_CHAR_LEN: usize = 3;

const UI_CALL_NAMES: &[&str] = &[
    "success", "error", "warning", "info", "message",
    "flash", "notify", "alert", "add_message", "add_error",
    "add_warning", "add_success",
];

const LOG_METHOD_NAMES: &[&str] = &[
    "debug", "info", "warning", "warn", "error", "critical",
    "exception", "log",
];

const LOG_OBJECT_NAMES: &[&str] = &[
    "logger", "logging", "log", "app", "current_app",
];

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
/// - contains at least one space, **or**
/// - is a single capitalized word (first character is uppercase).
fn is_prose(content: &str) -> bool {
    if content.is_empty() {
        return false;
    }
    if content.contains(' ') {
        return true;
    }
    content.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
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
    if content.starts_with('/')
        || content.starts_with("./")
        || content.starts_with("../")
    {
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
        && chars.iter().all(|c| c.is_uppercase() || *c == '_' || c.is_ascii_digit())
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
    LOG_OBJECT_NAMES.iter().any(|&n| obj_base.eq_ignore_ascii_case(n))
        || obj.to_ascii_lowercase().contains("logger")
        || obj.to_ascii_lowercase().contains("log")
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
            'r' | 'R' => { s = &s[1..]; }
            'u' | 'U' => { s = &s[1..]; }
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
        start: Position { line: s.row as u32, character: s.column as u32 },
        end: Position { line: e.row as u32, character: e.column as u32 },
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(src: &str) -> Vec<Diagnostic> {
        check_python(src.as_bytes(), &HashMap::new())
    }

    fn has(diags: &[Diagnostic]) -> bool {
        diags
            .iter()
            .any(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/hardcoded-string"))
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
        assert!(!has(&diags), "return of a call result is not a string literal");
    }

    // ── REQ-HARD-04: Gate 2 — prose shape ────────────────────────────────────

    #[test]
    fn req_hard_04_has_space_fires() {
        let diags = detect("def v():\n    return \"Order placed\"");
        assert!(has(&diags), "string with space is prose");
    }

    #[test]
    fn req_hard_04_single_capitalized_word_fires() {
        let diags = detect("def v():\n    return \"Checkout\"");
        assert!(has(&diags), "capitalized single word is prose");
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
        assert!(!has(&diags), "UPPER_CASE constant assignment should be silent");
    }

    #[test]
    fn req_hard_05_too_short_silent() {
        let diags = detect("def v():\n    return \"OK\"");
        assert!(!has(&diags), "short string below min length should be silent");
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
        assert!(!has(&diags), "logger.error arg should be silent as log call");
    }

    #[test]
    fn overlap_logging_warning_silent() {
        // "warning" is in both lists. logging.warning is a log call.
        let diags = detect("logging.warning(\"Deprecated API called\")");
        assert!(!has(&diags), "logging.warning arg should be silent as log call");
    }

    // ── Extra behavioural checks ──────────────────────────────────────────────

    #[test]
    fn extra_keywords_exclude_custom_translation_func() {
        let mut extra = HashMap::new();
        extra.insert("t".to_string(), TranslationFunc::Gettext);
        let diags = check_python(b"def v():\n    return t(\"Checkout\")", &extra);
        assert!(!has(&diags), "custom translation func should be treated as wrapped");
    }

    #[test]
    fn raise_inside_translation_call_silent() {
        // raise _(Error("...")) — contrived but ensures we don't double-flag
        let diags = detect("raise _(SomeError(\"Checkout\"))");
        assert!(!has(&diags), "raise wrapping a translation call should be silent");
    }

    #[test]
    fn multiple_findings_in_one_file() {
        let src = "def v():\n    return \"Order placed\"\ndef w():\n    return \"Save changes\"";
        let diags = detect(src);
        assert_eq!(diags.len(), 2, "should find two hardcoded strings");
    }
}
