use std::collections::HashMap;

use tower_lsp_server::ls_types::{Position, Range};
use tree_sitter::{Node, Parser};

use super::types::{TranslationCall, TranslationFunc, UnresolvedReason};

// ── Public API ────────────────────────────────────────────────────────────────

/// Extract all translation calls from Jinja2 source bytes.
///
/// Handles `{{ _("msg") }}` expression calls (REQ-EXT-08) and
/// `{% trans %}…{% endtrans %}` blocks with optional `{% pluralize %}` and
/// `context` (REQ-EXT-09..11).
pub fn extract(
    source: &[u8],
    extra_keywords: &HashMap<String, TranslationFunc>,
) -> Vec<TranslationCall> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_jinja2::LANGUAGE.into())
        .expect("tree-sitter-jinja2 load");
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
    match node.kind() {
        "print" => {
            if let Some(call) = extract_from_print(node, source, extra) {
                out.push(call);
            }
            // Don't recurse: print is a leaf for translation extraction.
        }
        "trans_statement" => {
            if let Some(call) = extract_trans(node, source) {
                out.push(call);
            }
            // Don't recurse: trans_statement is a leaf.
        }
        _ => {
            // Walk past all other nodes, including ERROR nodes (REQ-EXT-13).
            let mut cursor = node.walk();
            let children: Vec<_> = node.named_children(&mut cursor).collect();
            for child in children {
                walk(child, source, extra, out);
            }
        }
    }
}

// ── {{ expr }} extraction ─────────────────────────────────────────────────────

fn extract_from_print<'tree>(
    node: Node<'tree>,
    source: &[u8],
    extra: &HashMap<String, TranslationFunc>,
) -> Option<TranslationCall> {
    if let Some(call_node) = find_call_in_subtree(node) {
        match call_node.kind() {
            "function_call" => extract_func_call(call_node, source, extra),
            "method_call" => extract_method_call(call_node, source, extra),
            _ => None,
        }
    } else {
        None
    }
}

/// Depth-first search for the first `function_call` or `method_call` node
/// in the subtree rooted at `node`. Descends through filter nodes and other
/// wrappers so that `{{ _('x')|upper }}` (print > filter > function_call) is found.
fn find_call_in_subtree(node: Node) -> Option<Node> {
    let kind = node.kind();
    if kind == "function_call" || kind == "method_call" {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = find_call_in_subtree(child) {
            return Some(found);
        }
    }
    None
}

fn extract_func_call(
    node: Node,
    source: &[u8],
    extra: &HashMap<String, TranslationFunc>,
) -> Option<TranslationCall> {
    let name_node = node.child_by_field_name("name")?;
    let callee = name_node.utf8_text(source).ok()?;
    let func = TranslationFunc::resolve(callee, extra)?;
    let slots = collect_jinja_slots(node, source);
    Some(build_call(node, func, slots))
}

fn extract_method_call(
    node: Node,
    source: &[u8],
    extra: &HashMap<String, TranslationFunc>,
) -> Option<TranslationCall> {
    let method_node = node.child_by_field_name("method")?;
    let callee = method_node.utf8_text(source).ok()?;
    let func = TranslationFunc::resolve(callee, extra)?;
    let slots = collect_jinja_slots(node, source);
    Some(build_call(node, func, slots))
}

/// Unresolved slot carries the arg node's range so diagnostics can point at it.
enum SlotValue {
    Resolved(String, Range),
    Unresolved(Range),
}

fn collect_jinja_slots(node: Node, source: &[u8]) -> Vec<Option<SlotValue>> {
    let mut cursor = node.walk();
    node.children_by_field_name("positional_argument", &mut cursor)
        .map(|arg| {
            if arg.kind() == "string" {
                strip_jinja_string(arg, source)
                    .map(|s| SlotValue::Resolved(s, node_range(arg)))
            } else {
                Some(SlotValue::Unresolved(node_range(arg)))
            }
        })
        .collect()
}

fn build_call(node: Node, func: TranslationFunc, slots: Vec<Option<SlotValue>>) -> TranslationCall {
    let mut it = slots.into_iter();
    let domain = func
        .has_domain()
        .then(|| {
            it.next().flatten().and_then(|sv| match sv {
                SlotValue::Resolved(s, _) => Some(s),
                SlotValue::Unresolved(_) => None,
            })
        })
        .flatten();
    let msgctxt = func
        .has_context()
        .then(|| {
            it.next().flatten().and_then(|sv| match sv {
                SlotValue::Resolved(s, _) => Some(s),
                SlotValue::Unresolved(_) => None,
            })
        })
        .flatten();
    let (msgid, msgid_range, unresolved_reason, unresolved_arg_range) =
        match it.next().flatten() {
            Some(SlotValue::Resolved(s, r)) => (Some(s), Some(r), None, None),
            Some(SlotValue::Unresolved(r)) => {
                (None, None, Some(UnresolvedReason::NonConstant), Some(r))
            }
            None => (None, None, None, None),
        };
    let msgid_plural = func
        .has_plural()
        .then(|| {
            it.next().flatten().and_then(|sv| match sv {
                SlotValue::Resolved(s, _) => Some(s),
                SlotValue::Unresolved(_) => None,
            })
        })
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
        is_implicit_concat: false,
    }
}

// ── {% trans %} extraction ────────────────────────────────────────────────────

fn extract_trans(node: Node, source: &[u8]) -> Option<TranslationCall> {
    let mut cursor = node.walk();
    let children: Vec<_> = node.named_children(&mut cursor).collect();

    let trans_open = children.first().filter(|n| n.kind() == "trans_open")?;

    // Context from {% trans context "button" %}
    let msgctxt = trans_open
        .child_by_field_name("context")
        .and_then(|n| strip_jinja_string(n, source));

    // Partition body into singular (before pluralize) and plural (after pluralize).
    let pluralize_idx = children.iter().position(|n| n.kind() == "pluralize_clause");
    let close_idx = children
        .iter()
        .position(|n| n.kind() == "trans_close")
        .unwrap_or(children.len());

    let singular_end = pluralize_idx.unwrap_or(close_idx);
    // Skip trans_open (index 0) and up to pluralize/close.
    let singular_nodes = &children[1..singular_end];
    let plural_nodes = pluralize_idx.map(|pi| &children[pi + 1..close_idx]);

    let msgid = build_trans_body(singular_nodes, source);
    let msgid_plural = plural_nodes.map(|body| build_trans_body(body, source));

    let func = match (msgid_plural.is_some(), msgctxt.is_some()) {
        (false, false) => TranslationFunc::Gettext,
        (false, true) => TranslationFunc::PGettext,
        (true, false) => TranslationFunc::NGettext,
        (true, true) => TranslationFunc::NPGettext,
    };

    let msgid_range = singular_nodes.first().and_then(|first| {
        singular_nodes.last().map(|last| Range {
            start: node_range(*first).start,
            end: node_range(*last).end,
        })
    });

    Some(TranslationCall {
        func,
        msgid: Some(msgid),
        msgid_plural,
        msgctxt,
        domain: None,
        range: node_range(node),
        msgid_range,
        unresolved_reason: None,
        unresolved_arg_range: None,
        is_implicit_concat: false,
    })
}

/// Reconstruct the message body from `text` and `print` nodes inside a trans block.
///
/// `{{ count }}` expressions are normalized to `%(count)s` per the Babel convention.
/// Leading/trailing whitespace is stripped and internal whitespace runs are collapsed
/// to a single space, matching Babel's own trans-block extraction behaviour.
fn build_trans_body(nodes: &[Node], source: &[u8]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node.kind() {
            "text" => {
                if let Ok(t) = node.utf8_text(source) {
                    out.push_str(t);
                }
            }
            "print" => out.push_str(&normalize_print_placeholder(*node, source)),
            _ => {}
        }
    }
    // Normalize whitespace: strip leading/trailing, collapse internal runs.
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Convert `{{ name }}` to `%(name)s` for the msgid body.
fn normalize_print_placeholder(node: Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "identifier" {
            if let Ok(name) = child.utf8_text(source) {
                return format!("%({})s", name);
            }
        }
    }
    // Fallback: emit the raw source text of the print expression.
    node.utf8_text(source).unwrap_or("").to_string()
}

// ── String extraction ─────────────────────────────────────────────────────────

/// Strip surrounding quotes from a Jinja2 string literal node.
///
/// Jinja strings have no prefix letters; only `"…"` and `'…'` forms.
fn strip_jinja_string(node: Node, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?;
    let quote = match text.chars().next()? {
        c @ ('"' | '\'') => c,
        _ => return None,
    };
    if text.len() < 2 || !text.ends_with(quote) {
        return None;
    }
    Some(unescape_jinja(&text[1..text.len() - 1]))
}

fn unescape_jinja(s: &str) -> String {
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

    // REQ-EXT-08 — Jinja expression calls extract like Python calls

    #[test]
    fn req_ext_08_simple_call_extracts() {
        let calls = ex(r#"{{ _("Your cart") }}"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].func, TranslationFunc::Gettext);
        assert_eq!(calls[0].msgid.as_deref(), Some("Your cart"));
    }

    #[test]
    fn req_ext_08_pgettext_call() {
        let calls = ex(r#"{{ pgettext("button", "Save") }}"#);
        assert_eq!(calls[0].func, TranslationFunc::PGettext);
        assert_eq!(calls[0].msgctxt.as_deref(), Some("button"));
        assert_eq!(calls[0].msgid.as_deref(), Some("Save"));
    }

    #[test]
    fn req_ext_08_attribute_callee() {
        let calls = ex(r#"{{ gettext.gettext("Hello") }}"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello"));
    }

    #[test]
    fn req_ext_08_unrecognized_call_ignored() {
        let calls = ex(r#"{{ my_fn("Hello") }}"#);
        assert!(calls.is_empty());
    }

    // REQ-EXT-09 — {% trans %}…{% endtrans %} body becomes the msgid

    #[test]
    fn req_ext_09_trans_block_body_is_msgid() {
        let calls = ex("{% trans %}One item{% endtrans %}");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].func, TranslationFunc::Gettext);
        assert_eq!(calls[0].msgid.as_deref(), Some("One item"));
    }

    #[test]
    fn req_ext_09_trans_block_range_spans_whole_statement() {
        let calls = ex("{% trans %}One item{% endtrans %}");
        assert_eq!(calls[0].range.start.character, 0);
    }

    // REQ-EXT-10 — {% pluralize %} splits into NGettext

    #[test]
    fn req_ext_10_pluralize_splits_into_ngettext() {
        let calls = ex("{% trans count=n %}One item{% pluralize %}{{ count }} items{% endtrans %}");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].func, TranslationFunc::NGettext);
        assert_eq!(calls[0].msgid.as_deref(), Some("One item"));
        assert_eq!(calls[0].msgid_plural.as_deref(), Some("%(count)s items"));
    }

    #[test]
    fn req_ext_10_plural_without_bindings() {
        let calls = ex("{% trans %}Singular{% pluralize %}Plural{% endtrans %}");
        assert_eq!(calls[0].func, TranslationFunc::NGettext);
        assert_eq!(calls[0].msgid.as_deref(), Some("Singular"));
        assert_eq!(calls[0].msgid_plural.as_deref(), Some("Plural"));
    }

    #[test]
    fn trans_empty_body_emits_empty_msgid() {
        // Spec §10: empty body → msgid ""; the empty-msgid check belongs to F03.
        let calls = ex("{% trans %}{% endtrans %}");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].msgid.as_deref(), Some(""));
    }

    // REQ-EXT-11 — context and trans bindings

    #[test]
    fn req_ext_11_context_sets_msgctxt() {
        let calls = ex(r#"{% trans context "button" %}Save{% endtrans %}"#);
        assert_eq!(calls[0].func, TranslationFunc::PGettext);
        assert_eq!(calls[0].msgctxt.as_deref(), Some("button"));
        assert_eq!(calls[0].msgid.as_deref(), Some("Save"));
    }

    #[test]
    fn req_ext_11_context_with_plural_is_npgettext() {
        let calls = ex(r#"{% trans context "ctx" %}One{% pluralize %}Many{% endtrans %}"#);
        assert_eq!(calls[0].func, TranslationFunc::NPGettext);
        assert_eq!(calls[0].msgctxt.as_deref(), Some("ctx"));
        assert_eq!(calls[0].msgid.as_deref(), Some("One"));
        assert_eq!(calls[0].msgid_plural.as_deref(), Some("Many"));
    }

    // REQ-EXT-12 — non-literal arg yields msgid: None and sets unresolved_reason

    #[test]
    fn req_ext_12_variable_arg_is_none() {
        let calls = ex("{{ _(label) }}");
        assert_eq!(calls.len(), 1);
        assert!(calls[0].msgid.is_none());
        assert!(calls[0].msgid_range.is_none());
    }

    #[test]
    fn req_ext_12_variable_arg_sets_unresolved_reason() {
        use super::super::types::UnresolvedReason;
        let calls = ex("{{ _(label) }}");
        assert_eq!(calls[0].unresolved_reason, Some(UnresolvedReason::NonConstant));
        assert!(calls[0].unresolved_arg_range.is_some());
    }

    // babel-lsp-ff2 — filtered calls are extracted ({{ _('x')|upper }})

    #[test]
    fn filtered_call_is_extracted() {
        let calls = ex(r#"{{ _("Hello")|upper }}"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello"));
    }

    #[test]
    fn double_filtered_call_is_extracted() {
        let calls = ex(r#"{{ _("Hello")|upper|trim }}"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello"));
    }

    // REQ-EXT-13 — walk past ERROR nodes

    #[test]
    fn req_ext_13_walks_past_error_nodes() {
        // Unknown tag forces an ERROR node, but the valid trans_statement inside
        // it is still a named child — the walker descends past ERROR and extracts it.
        let src = "{% unknown_tag %}\n{% trans %}Valid{% endtrans %}";
        let calls = ex(src);
        assert!(calls.iter().any(|c| c.msgid.as_deref() == Some("Valid")));
    }

    // babel-lsp-71h — trans block body whitespace normalization

    #[test]
    fn trans_body_leading_trailing_whitespace_stripped() {
        let calls = ex("{% trans %}  Hello world  {% endtrans %}");
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello world"));
    }

    #[test]
    fn trans_body_internal_whitespace_collapsed() {
        let calls = ex("{% trans %}Hello   world{% endtrans %}");
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello world"));
    }

    #[test]
    fn trans_body_multiline_whitespace_collapsed() {
        let calls = ex("{% trans %}\n  Hello\n  world\n{% endtrans %}");
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello world"));
    }

    // babel-lsp-b4u — trans placeholder substitution

    #[test]
    fn trans_placeholder_substituted_in_body() {
        let calls = ex("{% trans name=user.name %}Hello {{ name }}{% endtrans %}");
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello %(name)s"));
    }

    // Extra_keywords

    #[test]
    fn extra_keywords_recognized_in_jinja() {
        let mut extra = HashMap::new();
        extra.insert("t".to_string(), TranslationFunc::Gettext);
        let calls = extract(r#"{{ t("Hello") }}"#.as_bytes(), &extra);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].msgid.as_deref(), Some("Hello"));
    }

    // Multiple calls in one template

    #[test]
    fn multiple_calls_extracted() {
        let src = r#"
{{ _("Your cart") }}
{% trans %}Welcome{% endtrans %}
{{ pgettext("nav", "Home") }}
"#;
        let calls = ex(src);
        assert_eq!(calls.len(), 3);
    }
}
