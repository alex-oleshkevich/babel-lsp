use std::collections::HashMap;

use tower_lsp_server::ls_types::{Position, Range};
use tree_sitter::{Node, Parser};

use super::types::{TranslationCall, TranslationFunc, UnresolvedReason};

// ── Public API ────────────────────────────────────────────────────────────────

/// Extract all translation calls from Python source bytes.
///
/// `extra_keywords` maps project-defined callee names to their variant
/// (REQ-EXT-03). Pass an empty map for the built-in table only.
pub fn extract(
    source: &[u8],
    extra_keywords: &HashMap<String, TranslationFunc>,
) -> Vec<TranslationCall> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .expect("tree-sitter-python load");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    let mut out = vec![];
    walk(tree.root_node(), source, extra_keywords, &mut out);
    out
}

// ── Tree walk ─────────────────────────────────────────────────────────────────

fn walk<'tree>(
    node: Node<'tree>,
    source: &[u8],
    extra: &HashMap<String, TranslationFunc>,
    out: &mut Vec<TranslationCall>,
) {
    if node.kind() == "call" {
        if let Some(call) = extract_call(node, source, extra) {
            out.push(call);
        }
    }
    // Descend into all named children, including past ERROR nodes (REQ-EXT-13).
    let mut cursor = node.walk();
    let children: Vec<_> = node.named_children(&mut cursor).collect();
    for child in children {
        walk(child, source, extra, out);
    }
}

fn extract_call<'tree>(
    node: Node<'tree>,
    source: &[u8],
    extra: &HashMap<String, TranslationFunc>,
) -> Option<TranslationCall> {
    let func_node = node.child_by_field_name("function")?;
    let callee = resolve_callee(func_node, source)?;
    let func = TranslationFunc::resolve(&callee, extra)?;

    let args_node = node.child_by_field_name("arguments")?;
    let slots = collect_slots(args_node, source);
    Some(build_call(node, func, slots))
}

// ── Callee resolution ─────────────────────────────────────────────────────────

/// Returns the trailing identifier from `identifier` or `attribute` nodes.
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

// ── Argument slot collection ──────────────────────────────────────────────────

/// A resolved or unresolved positional argument slot.
///
/// `None` at the slot level means the argument was absent (iterator exhausted).
enum SlotValue {
    /// A regular string literal with its range.
    Resolved(String, Range),
    /// An implicit string concatenation (`"a" "b"`) — resolved but flagged for
    /// `msg/implicit-concat`.
    Concat(String, Range),
    /// A non-literal argument: carries why it couldn't be resolved and its range
    /// so the diagnostic squiggle can point at the right node.
    Unresolved(UnresolvedReason, Range),
}

impl SlotValue {
    /// Extract the string content, returning `None` for unresolved slots.
    fn into_string(self) -> Option<String> {
        match self {
            Self::Resolved(s, _) | Self::Concat(s, _) => Some(s),
            Self::Unresolved(_, _) => None,
        }
    }
}

type Slot = Option<SlotValue>;

fn collect_slots(args_node: Node, source: &[u8]) -> Vec<Slot> {
    let mut cursor = args_node.walk();
    let mut slots: Vec<Slot> = vec![];
    for child in args_node.named_children(&mut cursor) {
        match child.kind() {
            "keyword_argument" => {} // positional slot count unaffected
            "string" => match extract_string(child, source) {
                Some(s) => slots.push(Some(SlotValue::Resolved(s, node_range(child)))),
                None => {
                    let reason = fstring_reason(child, source);
                    slots.push(Some(SlotValue::Unresolved(reason, node_range(child))));
                }
            },
            "concatenated_string" => {
                slots.push(Some(extract_concatenated(child, source)));
            }
            "call" => {
                let reason = if is_format_method_call(child, source) {
                    UnresolvedReason::FormatBeforeCall
                } else {
                    UnresolvedReason::NonConstant
                };
                slots.push(Some(SlotValue::Unresolved(reason, node_range(child))));
            }
            "binary_operator" => {
                let reason = if is_percent_format(child, source) {
                    UnresolvedReason::FormatBeforeCall
                } else {
                    UnresolvedReason::NonConstant
                };
                slots.push(Some(SlotValue::Unresolved(reason, node_range(child))));
            }
            _ => slots.push(Some(SlotValue::Unresolved(
                UnresolvedReason::NonConstant,
                node_range(child),
            ))),
        }
    }
    slots
}

fn build_call(node: Node, func: TranslationFunc, slots: Vec<Slot>) -> TranslationCall {
    let mut it = slots.into_iter();
    let domain = func
        .has_domain()
        .then(|| it.next().flatten().and_then(|sv| sv.into_string()))
        .flatten();
    let msgctxt = func
        .has_context()
        .then(|| it.next().flatten().and_then(|sv| sv.into_string()))
        .flatten();
    let (msgid, msgid_range, unresolved_reason, unresolved_arg_range, is_implicit_concat) =
        match it.next().flatten() {
            Some(SlotValue::Resolved(s, r)) => (Some(s), Some(r), None, None, false),
            Some(SlotValue::Concat(s, r)) => (Some(s), Some(r), None, None, true),
            Some(SlotValue::Unresolved(reason, r)) => (None, None, Some(reason), Some(r), false),
            None => (None, None, None, None, false),
        };
    let msgid_plural = func
        .has_plural()
        .then(|| it.next().flatten().and_then(|sv| sv.into_string()))
        .flatten();
    TranslationCall {
        func,
        msgid,
        msgid_plural,
        msgctxt,
        domain,
        range: node_range(node),
        msgid_range,
        unresolved_reason,
        unresolved_arg_range,
        is_implicit_concat,
    }
}

// ── Argument shape detection ──────────────────────────────────────────────────

/// Classify a `"string"` node that `extract_string` rejected as f-string vs other.
fn fstring_reason(node: Node, source: &[u8]) -> UnresolvedReason {
    let text = node.utf8_text(source).unwrap_or_default();
    let lower = text.to_ascii_lowercase();
    if lower.starts_with('f') || lower.starts_with("rf") || lower.starts_with("fr") {
        UnresolvedReason::FString
    } else {
        UnresolvedReason::NonConstant
    }
}

/// `true` if `node` is a call whose callee attribute is literally `"format"`.
fn is_format_method_call(node: Node, source: &[u8]) -> bool {
    node.child_by_field_name("function")
        .filter(|f| f.kind() == "attribute")
        .and_then(|f| f.child_by_field_name("attribute"))
        .and_then(|a| a.utf8_text(source).ok())
        == Some("format")
}

/// `true` if `node` is a binary expression whose operator is `%`.
fn is_percent_format(node: Node, source: &[u8]) -> bool {
    node.child_by_field_name("operator")
        .and_then(|op| op.utf8_text(source).ok())
        == Some("%")
}

// ── String extraction ─────────────────────────────────────────────────────────

fn extract_string(node: Node, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?;
    strip_python_string(text)
}

fn extract_concatenated(node: Node, source: &[u8]) -> SlotValue {
    let range = node_range(node);
    let mut cursor = node.walk();
    let parts: Vec<_> = node.named_children(&mut cursor).collect();
    let mut joined = String::new();
    for part in &parts {
        if part.kind() != "string" {
            return SlotValue::Unresolved(UnresolvedReason::NonConstant, range);
        }
        match extract_string(*part, source) {
            Some(s) => joined.push_str(&s),
            None => {
                return SlotValue::Unresolved(fstring_reason(*part, source), range);
            }
        }
    }
    SlotValue::Concat(joined, range)
}

/// Strip the outer prefix and quotes from a Python string literal source text.
///
/// Returns `None` for f-strings (`f"…"`) and byte strings (`b"…"`) — these
/// are not valid msgids.  Raw strings (`r"…"`) are accepted; their content is
/// not unescaped.
fn strip_python_string(text: &str) -> Option<String> {
    let mut s = text;
    let mut raw = false;
    // Validate and consume the prefix (r/R/u/U allowed; b/B/f/F → reject).
    loop {
        match s.chars().next()? {
            'r' | 'R' => {
                raw = true;
                s = &s[1..];
            }
            'u' | 'U' => {
                s = &s[1..];
            }
            'f' | 'F' | 'b' | 'B' => return None,
            _ => break,
        }
    }
    // Detect and strip quote delimiters.
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
    if raw {
        Some(content.to_string())
    } else {
        Some(unescape_python(content))
    }
}

fn unescape_python(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('\\') => out.push('\\'),
            Some('0') => out.push('\0'),
            // Line continuation: backslash followed by a newline — remove both
            // and skip any leading whitespace on the continuation line.
            Some('\n') => {
                while let Some(&next) = chars.as_str().as_bytes().first() {
                    if next == b' ' || next == b'\t' {
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            Some('\r') => {
                // Handle \r\n line endings for line continuation.
                if chars.as_str().starts_with('\n') {
                    chars.next();
                }
                while let Some(&next) = chars.as_str().as_bytes().first() {
                    if next == b' ' || next == b'\t' {
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            Some(c) => {
                out.push('\\');
                out.push(c);
            }
            None => out.push('\\'),
        }
    }
    out
}

// ── Utility ───────────────────────────────────────────────────────────────────

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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(src: &str) -> Vec<TranslationCall> {
        extract(src.as_bytes(), &Default::default())
    }

    fn ex_with_extra(src: &str, extra: HashMap<String, TranslationFunc>) -> Vec<TranslationCall> {
        extract(src.as_bytes(), &extra)
    }

    // REQ-EXT-01 — variant table maps each callee to its layout

    #[test]
    fn req_ext_01_gettext() {
        let calls = ex(r#"_("Hello")"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].func, TranslationFunc::Gettext);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello"));
    }

    #[test]
    fn req_ext_01_ngettext() {
        let calls = ex(r#"ngettext("item", "items", n)"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].func, TranslationFunc::NGettext);
        assert_eq!(calls[0].msgid.as_deref(), Some("item"));
        assert_eq!(calls[0].msgid_plural.as_deref(), Some("items"));
    }

    #[test]
    fn req_ext_01_pgettext() {
        let calls = ex(r#"pgettext("button", "Save")"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].func, TranslationFunc::PGettext);
        assert_eq!(calls[0].msgctxt.as_deref(), Some("button"));
        assert_eq!(calls[0].msgid.as_deref(), Some("Save"));
    }

    #[test]
    fn req_ext_01_npgettext() {
        let calls = ex(r#"npgettext("ctx", "item", "items", n)"#);
        assert_eq!(calls[0].func, TranslationFunc::NPGettext);
        assert_eq!(calls[0].msgctxt.as_deref(), Some("ctx"));
        assert_eq!(calls[0].msgid.as_deref(), Some("item"));
        assert_eq!(calls[0].msgid_plural.as_deref(), Some("items"));
    }

    #[test]
    fn req_ext_01_dgettext() {
        let calls = ex(r#"dgettext("admin", "Delete")"#);
        assert_eq!(calls[0].func, TranslationFunc::DGettext);
        assert_eq!(calls[0].domain.as_deref(), Some("admin"));
        assert_eq!(calls[0].msgid.as_deref(), Some("Delete"));
    }

    #[test]
    fn req_ext_01_dngettext() {
        let calls = ex(r#"dngettext("admin", "item", "items", n)"#);
        assert_eq!(calls[0].func, TranslationFunc::DNGettext);
        assert_eq!(calls[0].domain.as_deref(), Some("admin"));
        assert_eq!(calls[0].msgid.as_deref(), Some("item"));
        assert_eq!(calls[0].msgid_plural.as_deref(), Some("items"));
    }

    #[test]
    fn req_ext_01_dpgettext() {
        let calls = ex(r#"dpgettext("admin", "button", "Save")"#);
        assert_eq!(calls[0].func, TranslationFunc::DPGettext);
        assert_eq!(calls[0].domain.as_deref(), Some("admin"));
        assert_eq!(calls[0].msgctxt.as_deref(), Some("button"));
        assert_eq!(calls[0].msgid.as_deref(), Some("Save"));
    }

    #[test]
    fn req_ext_01_dnpgettext() {
        let calls = ex(r#"dnpgettext("admin", "ctx", "item", "items", n)"#);
        assert_eq!(calls[0].func, TranslationFunc::DNPGettext);
        assert_eq!(calls[0].domain.as_deref(), Some("admin"));
        assert_eq!(calls[0].msgctxt.as_deref(), Some("ctx"));
        assert_eq!(calls[0].msgid.as_deref(), Some("item"));
        assert_eq!(calls[0].msgid_plural.as_deref(), Some("items"));
    }

    // REQ-EXT-02 — aliases and lazy/u-forms collapse onto base variants

    #[test]
    fn req_ext_02_aliases_and_lazy_forms_collapse() {
        for callee in &["gettext", "gettext_lazy", "ugettext", "ugettext_lazy"] {
            let src = format!(r#"{}("Hello")"#, callee);
            let calls = ex(&src);
            assert_eq!(calls.len(), 1, "{callee}");
            assert_eq!(calls[0].func, TranslationFunc::Gettext, "{callee}");
        }
        for callee in &["ngettext", "ngettext_lazy", "ungettext", "ungettext_lazy"] {
            let src = format!(r#"{}("a", "b", n)"#, callee);
            let calls = ex(&src);
            assert_eq!(calls[0].func, TranslationFunc::NGettext, "{callee}");
        }
        for callee in &["pgettext", "pgettext_lazy"] {
            let src = format!(r#"{}("ctx", "msg")"#, callee);
            let calls = ex(&src);
            assert_eq!(calls[0].func, TranslationFunc::PGettext, "{callee}");
        }
    }

    // REQ-EXT-03 — extra_keywords extends the table

    #[test]
    fn req_ext_03_extra_keywords_extends_table() {
        let mut extra = HashMap::new();
        extra.insert("tr".to_string(), TranslationFunc::Gettext);
        let calls = ex_with_extra(r#"tr("Checkout")"#, extra);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].func, TranslationFunc::Gettext);
        assert_eq!(calls[0].msgid.as_deref(), Some("Checkout"));
    }

    #[test]
    fn req_ext_03_unmapped_name_yields_nothing() {
        let calls = ex(r#"my_gettext("Hello")"#);
        assert!(calls.is_empty());
    }

    // REQ-EXT-04 — resolve bare and attribute callees

    #[test]
    fn req_ext_04_resolves_bare_callee() {
        let calls = ex(r#"gettext("Hello")"#);
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn req_ext_04_resolves_attribute_callee() {
        let calls = ex(r#"gettext.gettext("Hello")"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello"));
    }

    #[test]
    fn req_ext_04_lookalike_name_ignored() {
        let calls = ex(r#"my_gettext("Hello")"#);
        assert!(calls.is_empty());
    }

    // REQ-EXT-05 — string literal types

    #[test]
    fn req_ext_05_single_quoted_string() {
        let calls = ex("_('Hello')");
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello"));
    }

    #[test]
    fn req_ext_05_triple_double_quoted() {
        let calls = ex(r#"_("""Hello""")"#);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello"));
    }

    #[test]
    fn req_ext_05_triple_single_quoted() {
        let calls = ex("_('''Hello''')");
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello"));
    }

    #[test]
    fn req_ext_05_u_prefix_accepted() {
        let calls = ex(r#"_(u"Hello")"#);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello"));
    }

    #[test]
    fn req_ext_05_r_prefix_accepted_no_unescape() {
        let calls = ex(r#"_(r"Hello\n")"#);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello\\n"));
    }

    #[test]
    fn req_ext_05_fstring_rejected() {
        let calls = ex(r#"_(f"Hello {user}")"#);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].msgid.is_none()); // unresolved
    }

    #[test]
    fn req_ext_05_bytes_rejected() {
        let calls = ex(r#"_(b"Hello")"#);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].msgid.is_none()); // unresolved
    }

    // REQ-EXT-06 — adjacent string literals join into one msgid

    #[test]
    fn req_ext_06_joins_adjacent_string_literals() {
        let calls = ex(r#"_("Order " "summary")"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].msgid.as_deref(), Some("Order summary"));
    }

    #[test]
    fn req_ext_06_msgid_range_spans_concatenation() {
        let calls = ex(r#"_("Order " "summary")"#);
        let r = calls[0].msgid_range.unwrap();
        // Range should start at the first string and end at the last
        assert_eq!(r.start.character, 2); // after _(
        assert!(r.end.character > r.start.character);
    }

    // REQ-EXT-07 — two ranges: call range and msgid_range

    #[test]
    fn req_ext_07_emits_translation_call_with_two_ranges() {
        let calls = ex(r#"_("Checkout")"#);
        let call = &calls[0];
        assert!(call.msgid_range.is_some());
        // call range starts at `_`, msgid_range starts at `"`
        assert_eq!(call.range.start.character, 0);
        assert_eq!(call.msgid_range.unwrap().start.character, 2);
    }

    // REQ-EXT-12 — non-literal first arg yields msgid: None

    #[test]
    fn req_ext_12_variable_arg_is_none() {
        let calls = ex(r#"_(label)"#);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].msgid.is_none());
        assert!(calls[0].msgid_range.is_none());
    }

    #[test]
    fn req_ext_12_fstring_arg_is_none() {
        let calls = ex(r#"_(f"Hello {user}")"#);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].msgid.is_none());
    }

    // REQ-EXT-13 — walk past ERROR nodes; partial extraction

    #[test]
    fn req_ext_13_walks_past_error_nodes() {
        // Unterminated call + valid call below: both are processed.
        let src = b"_(\ngettext('Valid')";
        let calls = extract(src, &Default::default());
        // gettext('Valid') should still be extracted
        assert!(calls.iter().any(|c| c.msgid.as_deref() == Some("Valid")));
    }

    // String escape handling

    #[test]
    fn unescape_newline_in_string() {
        let calls = ex(r#"_("Hello\nWorld")"#);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello\nWorld"));
    }

    #[test]
    fn unescape_tab_in_string() {
        let calls = ex(r#"_("Hello\tWorld")"#);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello\tWorld"));
    }

    // babel-lsp-cqd — backslash-newline line continuation inside string literals

    #[test]
    fn line_continuation_in_string_removed() {
        // "Hello \<newline>world" → "Hello world"
        let calls = ex("_(\"Hello \\\nworld\")");
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello world"));
    }

    #[test]
    fn line_continuation_strips_leading_whitespace_on_next_line() {
        // "Hello \<newline>    world" → "Hello world" (leading spaces on continuation removed)
        let calls = ex("_(\"Hello \\\n    world\")");
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello world"));
    }

    // Multiple calls in one file

    #[test]
    fn multiple_calls_extracted() {
        let src = r#"
a = _("Checkout")
b = pgettext("button", "Save")
c = ngettext("%(n)d item", "%(n)d items", n)
"#;
        let calls = ex(src);
        assert_eq!(calls.len(), 3);
    }

    // ── UnresolvedReason detection ────────────────────────────────────────────

    #[test]
    fn fstring_sets_fstring_reason() {
        let calls = ex(r#"_(f"Hello {user}")"#);
        assert_eq!(calls[0].unresolved_reason, Some(UnresolvedReason::FString));
        assert!(
            calls[0].unresolved_arg_range.is_some(),
            "unresolved range must be set"
        );
    }

    #[test]
    fn percent_format_sets_format_before_call_reason() {
        let calls = ex(r#"_("Hi %s" % name)"#);
        assert_eq!(
            calls[0].unresolved_reason,
            Some(UnresolvedReason::FormatBeforeCall)
        );
        assert!(calls[0].unresolved_arg_range.is_some());
    }

    #[test]
    fn format_method_sets_format_before_call_reason() {
        let calls = ex(r#"_("Hello {}".format(name))"#);
        assert_eq!(
            calls[0].unresolved_reason,
            Some(UnresolvedReason::FormatBeforeCall)
        );
    }

    #[test]
    fn variable_arg_sets_non_constant_reason() {
        let calls = ex(r#"_(label)"#);
        assert_eq!(
            calls[0].unresolved_reason,
            Some(UnresolvedReason::NonConstant)
        );
        assert!(calls[0].unresolved_arg_range.is_some());
    }

    #[test]
    fn regular_string_has_no_reason() {
        let calls = ex(r#"_("Hello")"#);
        assert!(calls[0].unresolved_reason.is_none());
        assert!(calls[0].unresolved_arg_range.is_none());
        assert!(!calls[0].is_implicit_concat);
    }

    // ── is_implicit_concat ────────────────────────────────────────────────────

    #[test]
    fn implicit_concat_sets_flag() {
        let calls = ex(r#"_("Hello " "World")"#);
        assert!(calls[0].is_implicit_concat);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello World"));
        assert!(calls[0].unresolved_reason.is_none());
    }

    #[test]
    fn single_string_does_not_set_concat_flag() {
        let calls = ex(r#"_("Hello")"#);
        assert!(!calls[0].is_implicit_concat);
    }

    // ── concatenated_string with f-string parts ───────────────────────────────

    #[test]
    fn concat_with_fstring_part_yields_unresolved_fstring() {
        // _('a' f'{x}') — the f-string part makes extraction impossible.
        let calls = ex(r#"_('a' f'{x}')"#);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].msgid.is_none());
        assert_eq!(calls[0].unresolved_reason, Some(UnresolvedReason::FString));
        assert!(calls[0].unresolved_arg_range.is_some());
        assert!(!calls[0].is_implicit_concat);
    }

    #[test]
    fn concat_fstring_leading_part_yields_unresolved_fstring() {
        // _(f'{x}' 'b') — f-string is the first part.
        let calls = ex(r#"_(f'{x}' 'b')"#);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].msgid.is_none());
        assert_eq!(calls[0].unresolved_reason, Some(UnresolvedReason::FString));
    }

    #[test]
    fn concat_all_plain_strings_resolves_normally() {
        // _('a' 'b') — no f-string, should still concatenate cleanly.
        let calls = ex(r#"_('a' 'b')"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].msgid.as_deref(), Some("ab"));
        assert!(calls[0].is_implicit_concat);
        assert!(calls[0].unresolved_reason.is_none());
    }
}
