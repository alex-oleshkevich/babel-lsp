use tower_lsp_server::ls_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Range};

use crate::catalog::index::{CatalogIndex, CatalogKey};
use crate::extract::types::{TranslationCall, UnresolvedReason};

/// Run all source-side diagnostic checks (msg/*) over a set of translation calls.
///
/// The returned `Diagnostic` slice has no URI — the caller binds them to the
/// document URI when publishing via `textDocument/publishDiagnostics`.
// Server wiring (publishDiagnostics) is task babel-lsp-1p7.4; suppress until then.
#[allow(dead_code)]
pub fn check_source(calls: &[TranslationCall], index: &CatalogIndex) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for call in calls {
        // ── Shape trio (REQ-DIAG-06) ─────────────────────────────────────────
        // Fires when the first argument is structurally unresolvable — we know
        // WHY without reading any msgid value.  Only one fires per call.
        if let Some(reason) = &call.unresolved_reason {
            let range = call.unresolved_arg_range.unwrap_or(call.range);
            let (code, severity, msg): (&str, DiagnosticSeverity, &str) = match reason {
                UnresolvedReason::FString => (
                    "msg/fstring-in-call",
                    DiagnosticSeverity::WARNING,
                    "f-string is interpolated before _(); the catalog never sees this template",
                ),
                UnresolvedReason::FormatBeforeCall => (
                    "msg/format-before-call",
                    DiagnosticSeverity::WARNING,
                    "the string is formatted before _(); pass placeholders through gettext instead",
                ),
                UnresolvedReason::NonConstant => (
                    "msg/non-constant-id",
                    DiagnosticSeverity::INFORMATION,
                    "msgid must be a string literal; pybabel extract cannot read this argument",
                ),
            };
            diags.push(make_diag(range, code, severity, msg));
        }

        // ── Msgid-based checks (REQ-DIAG-02: silent when unresolved) ─────────
        let Some(msgid) = &call.msgid else { continue };

        // msg/empty-id: "" is the gettext header sentinel
        if msgid.is_empty() {
            if let Some(r) = call.msgid_range {
                diags.push(make_diag(
                    r,
                    "msg/empty-id",
                    DiagnosticSeverity::WARNING,
                    "empty string is reserved for the gettext catalog header \
                     and never resolves to a translation",
                ));
            }
            continue; // no catalog lookup for empty msgid
        }

        // msg/implicit-concat: resolved but from adjacent literals
        if call.is_implicit_concat {
            if let Some(r) = call.msgid_range {
                diags.push(make_diag(
                    r,
                    "msg/implicit-concat",
                    DiagnosticSeverity::HINT,
                    "implicit string concatenation — prefer a single string literal",
                ));
            }
        }

        let key = CatalogKey { msgid: msgid.clone(), msgctxt: call.msgctxt.clone() };
        let in_po = !index.lookup(&key).is_empty();
        let in_pot = index.is_in_pot(&key);

        if !in_po && !in_pot {
            // msg/unknown-id (REQ-DIAG-05): needs both po and pot to vote
            if let Some(r) = call.msgid_range {
                diags.push(make_diag(
                    r,
                    "msg/unknown-id",
                    DiagnosticSeverity::WARNING,
                    format!("msgid '{}' is in no catalog or template", msgid),
                ));
            }
        } else {
            // msg/missing-in-locale: known but some locales have empty msgstr
            let missing = index.missing_locales(&key);
            if !missing.is_empty() {
                if let Some(r) = call.msgid_range {
                    diags.push(make_diag(
                        r,
                        "msg/missing-in-locale",
                        DiagnosticSeverity::INFORMATION,
                        format!("msgid '{}' is untranslated in: {}", msgid, missing.join(", ")),
                    ));
                }
            }
        }
    }
    diags
}

fn make_diag(
    range: Range,
    code: &str,
    severity: DiagnosticSeverity,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(severity),
        code: Some(NumberOrString::String(code.to_string())),
        code_description: None,
        source: Some("babel-lsp".to_string()),
        message: message.into(),
        related_information: None,
        tags: None,
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::catalog::index::{CatalogEntry, CatalogIndex, EntryFlags};
    use crate::extract::python;
    use crate::extract::types::TranslationFunc;

    fn no_extra() -> std::collections::HashMap<String, TranslationFunc> {
        std::collections::HashMap::new()
    }

    fn calls(src: &str) -> Vec<TranslationCall> {
        python::extract(src.as_bytes(), &no_extra())
    }

    fn empty_index() -> CatalogIndex {
        CatalogIndex::default()
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
            file_path: PathBuf::from("/locale/messages.po"),
            line: 1,
        }
    }

    fn codes(diags: &[Diagnostic]) -> Vec<&str> {
        diags
            .iter()
            .filter_map(|d| match d.code.as_ref()? {
                NumberOrString::String(s) => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }

    fn has_code(diags: &[Diagnostic], code: &str) -> bool {
        codes(diags).contains(&code)
    }

    // ── REQ-DIAG-06: shape trio ───────────────────────────────────────────────

    #[test]
    fn req_diag_06_fstring_in_call() {
        let c = calls(r#"_(f"Hello {user}")"#);
        let diags = check_source(&c, &empty_index());
        assert!(has_code(&diags, "msg/fstring-in-call"), "got: {:?}", codes(&diags));
    }

    #[test]
    fn req_diag_06_format_before_call_percent() {
        let c = calls(r#"_("Hi %s" % name)"#);
        let diags = check_source(&c, &empty_index());
        assert!(has_code(&diags, "msg/format-before-call"), "got: {:?}", codes(&diags));
    }

    #[test]
    fn req_diag_06_format_before_call_method() {
        let c = calls(r#"_("Hello {}".format(name))"#);
        let diags = check_source(&c, &empty_index());
        assert!(has_code(&diags, "msg/format-before-call"), "got: {:?}", codes(&diags));
    }

    #[test]
    fn req_diag_06_non_constant_id() {
        let c = calls(r#"_(label)"#);
        let diags = check_source(&c, &empty_index());
        assert!(has_code(&diags, "msg/non-constant-id"), "got: {:?}", codes(&diags));
    }

    // ── REQ-DIAG-02: unresolved call → msgid checks stay silent ──────────────

    #[test]
    fn req_diag_02_fstring_skips_unknown_id() {
        let c = calls(r#"_(f"Hello {user}")"#);
        let diags = check_source(&c, &empty_index());
        assert!(!has_code(&diags, "msg/unknown-id"), "got: {:?}", codes(&diags));
    }

    #[test]
    fn req_diag_02_non_constant_skips_msgid_checks() {
        let c = calls(r#"_(label)"#);
        let diags = check_source(&c, &empty_index());
        assert!(!has_code(&diags, "msg/unknown-id"));
        assert!(!has_code(&diags, "msg/missing-in-locale"));
    }

    // ── msg/empty-id ─────────────────────────────────────────────────────────

    #[test]
    fn empty_id_fires_on_empty_literal() {
        let c = calls(r#"_("")"#);
        let diags = check_source(&c, &empty_index());
        assert!(has_code(&diags, "msg/empty-id"), "got: {:?}", codes(&diags));
    }

    #[test]
    fn empty_id_skips_catalog_checks() {
        // Even though "Checkout" is in the index, _("") fires empty-id then stops —
        // no unknown-id and no missing-in-locale for the empty literal.
        let index = CatalogIndex::build(vec![make_entry("de", "Checkout", "Kasse")]);
        let c = calls(r#"_("")"#);
        let diags = check_source(&c, &index);
        assert!(!has_code(&diags, "msg/unknown-id"));
        assert!(!has_code(&diags, "msg/missing-in-locale"));
    }

    // ── msg/implicit-concat ───────────────────────────────────────────────────

    #[test]
    fn implicit_concat_fires_on_adjacent_literals() {
        let index = CatalogIndex::build(vec![make_entry("de", "Hello World", "Hallo Welt")]);
        let c = calls(r#"_("Hello " "World")"#);
        let diags = check_source(&c, &index);
        assert!(has_code(&diags, "msg/implicit-concat"), "got: {:?}", codes(&diags));
    }

    #[test]
    fn regular_string_does_not_fire_implicit_concat() {
        let index = CatalogIndex::build(vec![make_entry("de", "Hello", "Hallo")]);
        let c = calls(r#"_("Hello")"#);
        let diags = check_source(&c, &index);
        assert!(!has_code(&diags, "msg/implicit-concat"));
    }

    // ── REQ-DIAG-05: msg/unknown-id ──────────────────────────────────────────

    #[test]
    fn req_diag_05_unknown_id_fires_when_not_in_any_catalog() {
        let c = calls(r#"_("Unknown")"#);
        let diags = check_source(&c, &empty_index());
        assert!(has_code(&diags, "msg/unknown-id"), "got: {:?}", codes(&diags));
    }

    #[test]
    fn req_diag_05_unknown_id_silent_when_in_pot() {
        let mut pot = make_entry("", "Checkout", "");
        pot.file_path = "/locale/messages.pot".into();
        let index = CatalogIndex::build(vec![pot]);
        let c = calls(r#"_("Checkout")"#);
        let diags = check_source(&c, &index);
        assert!(!has_code(&diags, "msg/unknown-id"), "should be silent when key is in pot");
    }

    #[test]
    fn req_diag_05_unknown_id_silent_when_in_po() {
        let index = CatalogIndex::build(vec![make_entry("de", "Checkout", "Kasse")]);
        let c = calls(r#"_("Checkout")"#);
        let diags = check_source(&c, &index);
        assert!(!has_code(&diags, "msg/unknown-id"));
    }

    #[test]
    fn req_diag_05_squiggle_on_msgid_range() {
        let c = calls(r#"_("Chekout")"#);
        let diags = check_source(&c, &empty_index());
        let d = diags
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/unknown-id"))
            .expect("expected msg/unknown-id diagnostic");
        // msgid_range starts after `_(` at character 2
        assert_eq!(d.range.start.character, 2, "squiggle should start at the literal");
    }

    // ── msg/missing-in-locale ─────────────────────────────────────────────────

    #[test]
    fn missing_in_locale_fires_for_untranslated_locales() {
        let index = CatalogIndex::build(vec![
            make_entry("de", "Checkout", "Kasse"),
            make_entry("fr", "Checkout", ""), // empty → missing
        ]);
        let c = calls(r#"_("Checkout")"#);
        let diags = check_source(&c, &index);
        assert!(has_code(&diags, "msg/missing-in-locale"), "got: {:?}", codes(&diags));
        let msg = diags
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/missing-in-locale"))
            .unwrap()
            .message
            .clone();
        assert!(msg.contains("fr"), "message should name the missing locale, got: {}", msg);
    }

    #[test]
    fn missing_in_locale_silent_when_all_translated() {
        let index = CatalogIndex::build(vec![
            make_entry("de", "Checkout", "Kasse"),
            make_entry("fr", "Checkout", "Caisse"),
        ]);
        let c = calls(r#"_("Checkout")"#);
        let diags = check_source(&c, &index);
        assert!(!has_code(&diags, "msg/missing-in-locale"));
        assert!(!has_code(&diags, "msg/unknown-id"));
    }

    // ── Severity contract ─────────────────────────────────────────────────────

    #[test]
    fn severity_fstring_is_warning() {
        let c = calls(r#"_(f"Hello")"#);
        let d = check_source(&c, &empty_index())
            .into_iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/fstring-in-call"))
            .expect("expected msg/fstring-in-call");
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn severity_non_constant_is_information() {
        let c = calls(r#"_(label)"#);
        let d = check_source(&c, &empty_index())
            .into_iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/non-constant-id"))
            .expect("expected msg/non-constant-id");
        assert_eq!(d.severity, Some(DiagnosticSeverity::INFORMATION));
    }

    #[test]
    fn severity_implicit_concat_is_hint() {
        let index = CatalogIndex::build(vec![make_entry("de", "Hello World", "Hallo Welt")]);
        let c = calls(r#"_("Hello " "World")"#);
        let d = check_source(&c, &index)
            .into_iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/implicit-concat"))
            .expect("expected msg/implicit-concat");
        assert_eq!(d.severity, Some(DiagnosticSeverity::HINT));
    }

    #[test]
    fn severity_format_before_call_is_warning() {
        let c = calls(r#"_("Hi %s" % name)"#);
        let d = check_source(&c, &empty_index())
            .into_iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/format-before-call"))
            .expect("expected msg/format-before-call");
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn severity_empty_id_is_warning() {
        let c = calls(r#"_("")"#);
        let d = check_source(&c, &empty_index())
            .into_iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/empty-id"))
            .expect("expected msg/empty-id");
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn severity_unknown_id_is_warning() {
        let c = calls(r#"_("Unknown")"#);
        let d = check_source(&c, &empty_index())
            .into_iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/unknown-id"))
            .expect("expected msg/unknown-id");
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn severity_missing_in_locale_is_information() {
        let index = CatalogIndex::build(vec![make_entry("fr", "Checkout", "")]);
        let c = calls(r#"_("Checkout")"#);
        let d = check_source(&c, &index)
            .into_iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/missing-in-locale"))
            .expect("expected msg/missing-in-locale");
        assert_eq!(d.severity, Some(DiagnosticSeverity::INFORMATION));
    }

    // ── source field ──────────────────────────────────────────────────────────

    #[test]
    fn all_diagnostics_have_babel_lsp_source() {
        let c = calls(r#"_(f"Hello {user}")"#);
        let diags = check_source(&c, &empty_index());
        for d in &diags {
            assert_eq!(d.source.as_deref(), Some("babel-lsp"));
        }
    }
}
