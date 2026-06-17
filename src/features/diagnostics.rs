use std::collections::HashMap;

use tower_lsp_server::ls_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString,
    Position, Range, Uri,
};

use crate::catalog::index::{CatalogEntry, CatalogIndex, CatalogKey};
use crate::extract::types::{TranslationCall, UnresolvedReason};

/// Every diagnostic code the engine can emit — used by the CLI to validate
/// `--select`/`--ignore` arguments before running checks.
pub const KNOWN_CODES: &[&str] = &[
    "msg/empty-id",
    "msg/format-before-call",
    "msg/fstring-in-call",
    "msg/hardcoded-string",
    "msg/implicit-concat",
    "msg/missing-in-locale",
    "msg/non-constant-id",
    "msg/unknown-id",
    "po/accelerator-mismatch",
    "po/blank",
    "po/bracket-count",
    "po/double-space",
    "po/duplicate-id",
    "po/end-punctuation",
    "po/escape-mismatch",
    "po/extra-variable",
    "po/format-mismatch",
    "po/fuzzy",
    "po/header-missing",
    "po/missing-translation",
    "po/newline-count",
    "po/number-mismatch",
    "po/obsolete",
    "po/plural-count",
    "po/repeated-word",
    "po/same-plurals",
    "po/unchanged",
    "po/url-changed",
    "po/whitespace-edges",
    "po/xml-tag-mismatch",
    "proj/inconsistent-translation",
    "proj/missing-locale-file",
    "proj/unused-id",
];

/// Run all source-side diagnostic checks (msg/*) over a set of translation calls.
///
/// The returned `Diagnostic` slice has no URI — the caller binds them to the
/// document URI when publishing via `textDocument/publishDiagnostics`.
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

        let key = CatalogKey {
            msgid: msgid.clone(),
            msgctxt: call.msgctxt.clone(),
        };
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
        } else if in_po {
            // msg/missing-in-locale: known in .po but some locales have empty msgstr.
            // When the key is only in .pot (not yet merged into any .po), stay silent.
            let missing = index.missing_locales(&key);
            if !missing.is_empty() {
                if let Some(r) = call.msgid_range {
                    diags.push(make_diag(
                        r,
                        "msg/missing-in-locale",
                        DiagnosticSeverity::INFORMATION,
                        format!(
                            "msgid '{}' is untranslated in: {}",
                            msgid,
                            missing.join(", ")
                        ),
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

/// Apply per-rule select/ignore/severity config to a diagnostic list.
///
/// Rules (from E15 REQ-CFG-05):
/// - `select = ["all"]` passes every code; otherwise only listed codes survive.
/// - `ignore` drops any code listed there, after select.
/// - `severity` map overrides the severity for matching codes.
pub fn apply_diag_filter(
    diags: Vec<Diagnostic>,
    cfg: &crate::config::DiagnosticsConfig,
) -> Vec<Diagnostic> {
    let has_all = cfg.select.iter().any(|s| s == "all");
    diags
        .into_iter()
        .filter_map(|mut d| {
            let code = match &d.code {
                Some(NumberOrString::String(s)) => s.clone(),
                _ => return Some(d),
            };
            let selected = has_all || cfg.select.contains(&code);
            if !selected {
                return None;
            }
            if cfg.ignore.contains(&code) {
                return None;
            }
            if let Some(sev_str) = cfg.severity.get(&code) {
                d.severity = parse_severity(sev_str);
            }
            Some(d)
        })
        .collect()
}

fn parse_severity(s: &str) -> Option<DiagnosticSeverity> {
    match s.to_ascii_lowercase().as_str() {
        "error" => Some(DiagnosticSeverity::ERROR),
        "warning" | "warn" => Some(DiagnosticSeverity::WARNING),
        "information" | "info" => Some(DiagnosticSeverity::INFORMATION),
        "hint" => Some(DiagnosticSeverity::HINT),
        _ => None,
    }
}

pub fn check_catalog(
    entries: &[&CatalogEntry],
    file_uri: &Uri,
    index: &CatalogIndex,
) -> Vec<Diagnostic> {
    if entries.is_empty() {
        return vec![];
    }

    let mut diags = Vec::new();

    let header = entries
        .iter()
        .find(|e| e.msgid.is_empty() && !e.flags.obsolete)
        .copied();
    let nplurals: Option<usize> = header.and_then(|h| parse_nplurals(&h.msgstr.join("")));
    let has_plural_entries = entries
        .iter()
        .any(|e| e.msgid_plural.is_some() && !e.flags.obsolete);

    // po/header-missing
    match header {
        None => {
            diags.push(make_diag(
                Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 0,
                    },
                },
                "po/header-missing",
                DiagnosticSeverity::WARNING,
                "catalog has no header entry (missing msgid \"\")",
            ));
        }
        Some(h) => {
            let header_str = h.msgstr.join("");
            let lc = header_str.to_ascii_lowercase();
            if !lc.contains("content-type") || !lc.contains("charset=") {
                diags.push(make_diag(
                    entry_range(h),
                    "po/header-missing",
                    DiagnosticSeverity::WARNING,
                    "header lacks a Content-Type charset declaration",
                ));
            } else if has_plural_entries && !lc.contains("plural-forms") {
                diags.push(make_diag(
                    entry_range(h),
                    "po/header-missing",
                    DiagnosticSeverity::WARNING,
                    "header lacks a Plural-Forms declaration for a catalog with plural entries",
                ));
            }
        }
    }

    // Track first-seen entries for po/duplicate-id
    let mut seen_keys: HashMap<CatalogKey, &CatalogEntry> = HashMap::new();

    for entry in entries {
        // po/duplicate-id (non-obsolete; includes header so two headers are caught)
        if !entry.flags.obsolete {
            let key = entry.key();
            if let Some(first) = seen_keys.get(&key) {
                let mut d = make_diag(
                    entry_range(entry),
                    "po/duplicate-id",
                    DiagnosticSeverity::ERROR,
                    format!(
                        "duplicate msgid '{}' — first defined at line {}",
                        entry.msgid, first.line
                    ),
                );
                d.related_information = Some(vec![DiagnosticRelatedInformation {
                    location: Location {
                        uri: file_uri.clone(),
                        range: entry_range(first),
                    },
                    message: format!("first defined at line {}", first.line),
                }]);
                diags.push(d);
            } else {
                seen_keys.insert(key, entry);
            }
        }

        // Skip the catalog header for all per-entry content checks
        if entry.msgid.is_empty() {
            continue;
        }

        // po/obsolete (REQ-DIAG-09: needs .pot to vote; silent when no .pot loaded)
        if entry.flags.obsolete {
            if index.has_pot_entries() && !index.is_in_pot(&entry.key()) {
                diags.push(make_diag(
                    entry_range(entry),
                    "po/obsolete",
                    DiagnosticSeverity::HINT,
                    format!(
                        "obsolete entry '{}' is absent from the .pot template",
                        entry.msgid
                    ),
                ));
            }
            continue;
        }

        let all_empty = entry.msgstr.iter().all(|s| s.is_empty());

        // po/missing-translation
        if all_empty && !entry.flags.fuzzy {
            diags.push(make_diag(
                entry_range(entry),
                "po/missing-translation",
                DiagnosticSeverity::INFORMATION,
                format!("'{}' has no translation", entry.msgid),
            ));
        }

        // po/fuzzy
        if entry.flags.fuzzy {
            diags.push(make_diag(
                entry_range(entry),
                "po/fuzzy",
                DiagnosticSeverity::INFORMATION,
                "entry is marked fuzzy and needs review",
            ));
        }

        if all_empty {
            continue;
        }

        // po/blank: any form is non-empty but whitespace-only
        if entry
            .msgstr
            .iter()
            .any(|s| !s.is_empty() && s.trim().is_empty())
        {
            diags.push(make_diag(
                entry_range(entry),
                "po/blank",
                DiagnosticSeverity::WARNING,
                "translation is whitespace-only",
            ));
        }

        // po/plural-count (REQ-DIAG-08): only when header parses and entry is plural
        if let Some(n) = nplurals {
            if entry.msgid_plural.is_some() {
                let has_any = entry.msgstr.iter().any(|s| !s.is_empty());
                if has_any && entry.msgstr.len() != n {
                    diags.push(make_diag(
                        entry_range(entry),
                        "po/plural-count",
                        DiagnosticSeverity::WARNING,
                        format!(
                            "expected {} plural form(s) (nplurals={}), found {}",
                            n,
                            n,
                            entry.msgstr.len()
                        ),
                    ));
                }
            }
        }

        // po/same-plurals: all forms identical when nplurals > 1
        if entry.msgstr.len() > 1 {
            if let Some(first) = entry.msgstr.first() {
                if !first.is_empty() && entry.msgstr.iter().all(|s| s == first) {
                    diags.push(make_diag(
                        entry_range(entry),
                        "po/same-plurals",
                        DiagnosticSeverity::HINT,
                        "all plural forms are identical",
                    ));
                }
            }
        }

        // po/format-mismatch and po/extra-variable (REQ-DIAG-07)
        // Compare msgstr[0] vs msgid; msgstr[i>0] vs msgid_plural where available.
        let sources: Vec<&str> = match &entry.msgid_plural {
            Some(plural) => {
                let mut v = vec![entry.msgid.as_str()];
                v.extend(std::iter::repeat_n(
                    plural.as_str(),
                    entry.msgstr.len().saturating_sub(1),
                ));
                v
            }
            None => vec![entry.msgid.as_str()],
        };
        for (i, msgstr_form) in entry.msgstr.iter().enumerate() {
            if msgstr_form.is_empty() {
                continue;
            }
            let src = sources.get(i).copied().unwrap_or(entry.msgid.as_str());
            for (src_specs, str_specs, style) in [
                (
                    printf_specifiers(src),
                    printf_specifiers(msgstr_form),
                    "printf",
                ),
                (
                    brace_specifiers(src),
                    brace_specifiers(msgstr_form),
                    "brace",
                ),
            ] {
                if src_specs.is_empty() && str_specs.is_empty() {
                    continue;
                }
                let missing = specifiers_missing_from(&src_specs, &str_specs);
                let extra = specifiers_missing_from(&str_specs, &src_specs);
                if !missing.is_empty() {
                    diags.push(make_diag(
                        entry_range(entry),
                        "po/format-mismatch",
                        DiagnosticSeverity::WARNING,
                        format!(
                            "{} placeholder(s) {} missing from translation",
                            style,
                            missing.join(", ")
                        ),
                    ));
                }
                if !extra.is_empty() {
                    diags.push(make_diag(
                        entry_range(entry),
                        "po/extra-variable",
                        DiagnosticSeverity::WARNING,
                        format!(
                            "translation has extra {} placeholder(s) {} not in source",
                            style,
                            extra.join(", ")
                        ),
                    ));
                }
            }
        }

        // Single-form string-quality checks: first non-empty msgstr vs msgid
        let Some(msgstr) = entry.msgstr.iter().find(|s| !s.is_empty()) else {
            continue;
        };
        let msgstr = msgstr.as_str();
        let msgid = entry.msgid.as_str();

        // po/unchanged (REQ-DIAG-10): suppress when msgid has no lowercase letters
        if msgid.chars().any(|c| c.is_lowercase()) && msgstr == msgid {
            diags.push(make_diag(
                entry_range(entry),
                "po/unchanged",
                DiagnosticSeverity::HINT,
                "translation is identical to the source string",
            ));
        }

        // po/newline-count
        let id_nl = msgid.chars().filter(|c| *c == '\n').count();
        let str_nl = msgstr.chars().filter(|c| *c == '\n').count();
        if id_nl != str_nl {
            diags.push(make_diag(
                entry_range(entry),
                "po/newline-count",
                DiagnosticSeverity::WARNING,
                format!(
                    "\\n count: source has {}, translation has {}",
                    id_nl, str_nl
                ),
            ));
        }

        // po/whitespace-edges
        let id_lead: usize = msgid.chars().take_while(|c| c.is_whitespace()).count();
        let str_lead: usize = msgstr.chars().take_while(|c| c.is_whitespace()).count();
        let id_trail: usize = msgid
            .chars()
            .rev()
            .take_while(|c| c.is_whitespace())
            .count();
        let str_trail: usize = msgstr
            .chars()
            .rev()
            .take_while(|c| c.is_whitespace())
            .count();
        if id_lead != str_lead || id_trail != str_trail {
            diags.push(make_diag(
                entry_range(entry),
                "po/whitespace-edges",
                DiagnosticSeverity::INFORMATION,
                "leading or trailing whitespace differs from the source",
            ));
        }

        // po/end-punctuation
        if trailing_punctuation_differs(msgid, msgstr) {
            diags.push(make_diag(
                entry_range(entry),
                "po/end-punctuation",
                DiagnosticSeverity::INFORMATION,
                "trailing punctuation differs from the source",
            ));
        }

        // po/double-space
        if !msgid.contains("  ") && msgstr.contains("  ") {
            diags.push(make_diag(
                entry_range(entry),
                "po/double-space",
                DiagnosticSeverity::HINT,
                "translation contains a double space not present in the source",
            ));
        }

        // po/repeated-word
        if has_repeated_word(msgstr) && !has_repeated_word(msgid) {
            diags.push(make_diag(
                entry_range(entry),
                "po/repeated-word",
                DiagnosticSeverity::HINT,
                "translation repeats a word consecutively",
            ));
        }

        // po/escape-mismatch (exclude \n — owned by po/newline-count)
        let id_esc = collect_non_newline_escapes(msgid);
        let str_esc = collect_non_newline_escapes(msgstr);
        if id_esc != str_esc {
            diags.push(make_diag(
                entry_range(entry),
                "po/escape-mismatch",
                DiagnosticSeverity::WARNING,
                "backslash escape sequences differ from the source",
            ));
        }

        // po/bracket-count
        let (id_round, id_sq, id_curly) = bracket_counts(msgid);
        let (str_round, str_sq, str_curly) = bracket_counts(msgstr);
        if id_round != str_round || id_sq != str_sq || id_curly != str_curly {
            diags.push(make_diag(
                entry_range(entry),
                "po/bracket-count",
                DiagnosticSeverity::HINT,
                "bracket count differs from the source",
            ));
        }

        // po/accelerator-mismatch (REQ-DIAG-13: fires only when msgid has exactly one &)
        let id_accel = count_ampersand_accelerators(msgid);
        let str_accel = count_ampersand_accelerators(msgstr);
        if id_accel == 1 && str_accel != 1 {
            diags.push(make_diag(
                entry_range(entry),
                "po/accelerator-mismatch",
                DiagnosticSeverity::INFORMATION,
                format!(
                    "source has 1 accelerator marker (&), translation has {}",
                    str_accel
                ),
            ));
        }

        // po/xml-tag-mismatch
        let id_tags = xml_tag_counts(msgid);
        let str_tags = xml_tag_counts(msgstr);
        if id_tags != str_tags {
            diags.push(make_diag(
                entry_range(entry),
                "po/xml-tag-mismatch",
                DiagnosticSeverity::WARNING,
                "XML/HTML tag structure differs from the source",
            ));
        }

        // po/url-changed (OQ-DIAG-2: TLD/locale-only domain swap allowed)
        for url in extract_urls(msgid) {
            if !url_in_msgstr(&url, msgstr) {
                diags.push(make_diag(
                    entry_range(entry),
                    "po/url-changed",
                    DiagnosticSeverity::INFORMATION,
                    format!(
                        "URL '{}' from source is absent or path-altered in translation",
                        url
                    ),
                ));
            }
        }

        // po/number-mismatch
        let id_nums = extract_numbers(msgid);
        let str_nums = extract_numbers(msgstr);
        if id_nums != str_nums && !numbers_equivalent(&id_nums, &str_nums) {
            diags.push(make_diag(
                entry_range(entry),
                "po/number-mismatch",
                DiagnosticSeverity::INFORMATION,
                "numeric literals differ between source and translation",
            ));
        }
    }

    diags
}

/// Run project-level diagnostic checks (proj/*) across the full catalog index.
///
/// Returns diagnostics grouped by file URI — the caller merges them with
/// per-file catalog diagnostics before publishing.
pub fn check_project(
    index: &CatalogIndex,
    all_calls: &[crate::extract::types::TranslationCall],
) -> HashMap<Uri, Vec<Diagnostic>> {
    let mut out: HashMap<Uri, Vec<Diagnostic>> = HashMap::new();
    proj_inconsistent_translation(index, &mut out);
    proj_unused_id(index, all_calls, &mut out);
    proj_missing_locale_file(index, &mut out);
    out
}

fn proj_inconsistent_translation(index: &CatalogIndex, out: &mut HashMap<Uri, Vec<Diagnostic>>) {
    for key in index.all_msgids() {
        let entries = index.lookup(key);

        // Group non-fuzzy, non-obsolete, non-empty entries by locale.
        let mut by_locale: HashMap<&str, Vec<&CatalogEntry>> = HashMap::new();
        for e in entries {
            if e.flags.obsolete || e.flags.fuzzy {
                continue;
            }
            if e.msgstr.iter().all(|s| s.is_empty()) {
                continue;
            }
            by_locale.entry(e.locale.as_str()).or_default().push(e);
        }

        for locale_entries in by_locale.values() {
            if locale_entries.len() < 2 {
                continue;
            }
            let first = &locale_entries[0].msgstr;
            if locale_entries.iter().all(|e| &e.msgstr == first) {
                continue;
            }

            let related: Vec<DiagnosticRelatedInformation> = locale_entries
                .iter()
                .filter_map(|e| {
                    let uri = Uri::from_file_path(&e.file_path)?;
                    let label = e
                        .msgstr
                        .first()
                        .map(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(DiagnosticRelatedInformation {
                        location: Location {
                            uri,
                            range: entry_range(e),
                        },
                        message: label,
                    })
                })
                .collect();

            for e in locale_entries {
                let Some(uri) = Uri::from_file_path(&e.file_path) else {
                    continue;
                };
                out.entry(uri).or_default().push(Diagnostic {
                    range: entry_range(e),
                    severity: Some(DiagnosticSeverity::INFORMATION),
                    code: Some(NumberOrString::String(
                        "proj/inconsistent-translation".into(),
                    )),
                    code_description: None,
                    source: Some("babel-lsp".into()),
                    message: format!(
                        "conflicting translations for '{}' in locale '{}'",
                        key.msgid, e.locale
                    ),
                    related_information: Some(related.clone()),
                    tags: None,
                    data: None,
                });
            }
        }
    }
}

fn proj_unused_id(
    index: &CatalogIndex,
    all_calls: &[crate::extract::types::TranslationCall],
    out: &mut HashMap<Uri, Vec<Diagnostic>>,
) {
    // P4 gate: any unresolved call could match any id, so absence is unprovable.
    if all_calls.iter().any(|c| c.msgid.is_none()) {
        return;
    }

    let referenced: std::collections::HashSet<CatalogKey> = all_calls
        .iter()
        .filter_map(|c| {
            c.msgid.as_ref().map(|id| CatalogKey {
                msgid: id.clone(),
                msgctxt: c.msgctxt.clone(),
            })
        })
        .collect();

    for key in index.all_msgids() {
        if referenced.contains(key) {
            continue;
        }
        for entry in index.lookup(key) {
            if entry.flags.obsolete {
                continue;
            }
            let Some(uri) = Uri::from_file_path(&entry.file_path) else {
                continue;
            };
            out.entry(uri).or_default().push(make_diag(
                entry_range(entry),
                "proj/unused-id",
                DiagnosticSeverity::HINT,
                format!("msgid '{}' is not referenced in any source file", key.msgid),
            ));
        }
    }

    for key in index.all_pot_keys() {
        if referenced.contains(key) {
            continue;
        }
        let Some(entry) = index.lookup_pot(key) else {
            continue;
        };
        let Some(uri) = Uri::from_file_path(&entry.file_path) else {
            continue;
        };
        out.entry(uri).or_default().push(make_diag(
            entry_range(entry),
            "proj/unused-id",
            DiagnosticSeverity::HINT,
            format!("msgid '{}' is not referenced in any source file", key.msgid),
        ));
    }
}

fn proj_missing_locale_file(index: &CatalogIndex, out: &mut HashMap<Uri, Vec<Diagnostic>>) {
    // Build a map of domain → (set of locales that have entries, one example .po path).
    let mut by_domain: HashMap<String, (std::collections::BTreeSet<String>, std::path::PathBuf)> =
        HashMap::new();

    for key in index.all_msgids() {
        for entry in index.lookup(key) {
            if entry.flags.obsolete || entry.locale.is_empty() {
                continue;
            }
            let (locales, _) = by_domain
                .entry(entry.domain.clone())
                .or_insert_with(|| (Default::default(), entry.file_path.clone()));
            locales.insert(entry.locale.clone());
        }
    }

    // Build a per-domain pot file path map by iterating pot entries.
    let mut pot_path_by_domain: HashMap<&str, &std::path::Path> = HashMap::new();
    for key in index.all_pot_keys() {
        if let Some(entry) = index.lookup_pot(key) {
            pot_path_by_domain
                .entry(entry.domain.as_str())
                .or_insert(entry.file_path.as_path());
        }
    }

    let all_locales = index.all_locales();

    for (domain, (domain_locales, example_path)) in &by_domain {
        // Prefer anchoring to the .pot file for this domain; fall back to example .po path.
        let anchor_path: &std::path::Path = pot_path_by_domain
            .get(domain.as_str())
            .copied()
            .unwrap_or(example_path.as_path());
        let Some(uri) = Uri::from_file_path(anchor_path) else {
            continue;
        };
        for locale in all_locales {
            if domain_locales.contains(locale) {
                continue;
            }
            out.entry(uri.clone()).or_default().push(make_diag(
                Range::default(),
                "proj/missing-locale-file",
                DiagnosticSeverity::INFORMATION,
                format!("no '.po' for locale '{}' in domain '{}'", locale, domain),
            ));
        }
    }
}

// ── catalog helpers ───────────────────────────────────────────────────────────

fn entry_range(entry: &CatalogEntry) -> Range {
    let line = entry.line.saturating_sub(1);
    Range {
        start: Position { line, character: 0 },
        end: Position { line, character: 0 },
    }
}

fn parse_nplurals(s: &str) -> Option<usize> {
    let line = s.lines().find(|l| l.contains("Plural-Forms"))?;
    let after = line.split("nplurals=").nth(1)?.trim_start();
    after
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn printf_specifiers(s: &str) -> Vec<String> {
    let mut specs = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            continue;
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next(); // %% escape
            }
            Some('(') => {
                chars.next();
                let mut name = String::new();
                for ch in chars.by_ref() {
                    if ch == ')' {
                        break;
                    }
                    name.push(ch);
                }
                if let Some(conv) = chars.peek().copied() {
                    if "diouxXeEfFgGcrsab".contains(conv) {
                        specs.push(format!("%({}){}", name, conv));
                        chars.next();
                    }
                }
            }
            Some(conv) if "diouxXeEfFgGcrsab".contains(conv) => {
                specs.push(format!("%{}", conv));
                chars.next();
            }
            _ => {}
        }
    }
    specs
}

fn brace_specifiers(s: &str) -> Vec<String> {
    let mut specs = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            if chars.peek() == Some(&'{') {
                chars.next(); // {{ escape
                continue;
            }
            let mut content = String::new();
            for ch in chars.by_ref() {
                if ch == '}' {
                    break;
                }
                content.push(ch);
            }
            let name = content.split([':', '!']).next().unwrap_or("").trim();
            specs.push(format!("{{{}}}", name));
        } else if c == '}' && chars.peek() == Some(&'}') {
            chars.next(); // }} escape
        }
    }
    specs
}

/// Multiset difference: elements in `have` that are absent from (or exceed the count in) `got`.
fn specifiers_missing_from(have: &[String], got: &[String]) -> Vec<String> {
    let mut available: HashMap<&str, usize> = HashMap::new();
    for s in got {
        *available.entry(s.as_str()).or_default() += 1;
    }
    let mut missing = Vec::new();
    for s in have {
        match available.get_mut(s.as_str()) {
            Some(n) if *n > 0 => {
                *n -= 1;
            }
            _ => missing.push(s.clone()),
        }
    }
    missing
}

fn xml_tag_counts(s: &str) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '<' {
            continue;
        }
        if chars.peek() == Some(&'/') {
            chars.next();
        }
        let mut tag = String::new();
        while let Some(&ch) = chars.peek() {
            if matches!(ch, '>' | ' ' | '/' | '\n' | '\t') {
                break;
            }
            tag.push(ch);
            chars.next();
        }
        let tag = tag.to_ascii_lowercase();
        if !tag.is_empty()
            && tag
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == ':')
        {
            *counts.entry(tag).or_default() += 1;
        }
        for ch in chars.by_ref() {
            if ch == '>' {
                break;
            }
        }
    }
    counts
}

fn extract_urls(s: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut idx = 0;
    while idx < s.len() {
        let rest = &s[idx..];
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let end = rest
                .find(|c: char| {
                    c.is_ascii_whitespace() || matches!(c, ')' | ',' | '"' | '\'' | '<' | '>')
                })
                .unwrap_or(rest.len());
            let raw = &rest[..end];
            // Strip trailing ASCII punctuation that commonly appears in prose
            let trimmed =
                raw.trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '!' | '?'));
            urls.push(trimmed.to_string());
            idx += end;
        } else {
            idx += rest.chars().next().map_or(1, |c| c.len_utf8());
        }
    }
    urls
}

fn url_path_part(url: &str) -> &str {
    url.find("://")
        .map(|i| &url[i + 3..])
        .and_then(|after| after.find('/').map(|j| &after[j..]))
        .unwrap_or("")
}

fn url_in_msgstr(msgid_url: &str, msgstr: &str) -> bool {
    if msgstr.contains(msgid_url) {
        return true;
    }
    // TLD/locale-only domain swap: same path, different host → still ok (OQ-DIAG-2)
    let id_path = url_path_part(msgid_url);
    if !id_path.is_empty() {
        for str_url in extract_urls(msgstr) {
            if url_path_part(&str_url) == id_path {
                return true;
            }
        }
    }
    false
}

fn extract_numbers(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut nums = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let prev_alpha = i > 0 && chars[i - 1].is_alphabetic();
            if !prev_alpha {
                let start = i;
                // Consume digits, allowing commas as thousand separators
                // (e.g. "1,000" or "1,000,000")
                while i < chars.len() {
                    if chars[i].is_ascii_digit() {
                        i += 1;
                    } else if chars[i] == ','
                        && i + 1 < chars.len()
                        && chars[i + 1].is_ascii_digit()
                    {
                        // Comma followed by digit — treat as thousand separator
                        i += 1;
                    } else {
                        break;
                    }
                }
                let next_alpha = i < chars.len() && chars[i].is_alphabetic();
                if !next_alpha {
                    // Normalize: remove commas so "1,000" becomes "1000"
                    let raw: String = chars[start..i].iter().collect();
                    nums.push(raw.replace(',', ""));
                }
                continue;
            }
        }
        i += 1;
    }
    nums.sort();
    nums
}

/// Check if two number lists represent the same numeric values, tolerating locale
/// formatting differences (thousand separators, decimal comma vs period).
/// Strips commas and periods used as thousand separators before parsing as f64.
fn numbers_equivalent(a: &[String], b: &[String]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let parse = |s: &str| -> Option<f64> {
        // Remove thousand separators (commas) and try direct parse
        let cleaned = s.replace(',', "");
        cleaned.parse::<f64>().ok()
    };
    let mut a_vals: Vec<Option<f64>> = a.iter().map(|s| parse(s)).collect();
    let mut b_vals: Vec<Option<f64>> = b.iter().map(|s| parse(s)).collect();
    // If any fail to parse, fall back to string comparison (not equivalent)
    if a_vals.iter().any(|v| v.is_none()) || b_vals.iter().any(|v| v.is_none()) {
        return false;
    }
    a_vals.sort_by(|x, y| {
        x.unwrap()
            .partial_cmp(&y.unwrap())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    b_vals.sort_by(|x, y| {
        x.unwrap()
            .partial_cmp(&y.unwrap())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    a_vals
        .iter()
        .zip(b_vals.iter())
        .all(|(x, y)| (x.unwrap() - y.unwrap()).abs() < f64::EPSILON)
}

fn has_repeated_word(s: &str) -> bool {
    let words: Vec<&str> = s.split_whitespace().collect();
    words.windows(2).any(|w| {
        let a = w[0].trim_matches(|c: char| !c.is_alphanumeric());
        let b = w[1].trim_matches(|c: char| !c.is_alphanumeric());
        !a.is_empty() && !b.is_empty() && a.to_lowercase() == b.to_lowercase()
    })
}

fn collect_non_newline_escapes(s: &str) -> Vec<String> {
    let mut escapes = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek().copied() {
                Some('n') => {
                    chars.next(); // \n is owned by po/newline-count
                }
                Some(next) => {
                    escapes.push(format!("\\{}", next));
                    chars.next();
                }
                None => {}
            }
        }
    }
    escapes.sort();
    escapes
}

fn bracket_counts(s: &str) -> (usize, usize, usize) {
    let round = s.chars().filter(|c| matches!(c, '(' | ')')).count();
    let square = s.chars().filter(|c| matches!(c, '[' | ']')).count();
    let curly = s.chars().filter(|c| matches!(c, '{' | '}')).count();
    (round, square, curly)
}

fn count_ampersand_accelerators(s: &str) -> usize {
    s.chars().filter(|c| *c == '&').count()
}

fn trailing_punctuation_differs(msgid: &str, msgstr: &str) -> bool {
    const PUNCT: &[char] = &[
        '.', '!', '?', ':', ';', '\u{3002}', '\u{FF1F}', '\u{FF01}', '\u{FF1A}', '\u{FF1B}',
    ];
    let id_end = msgid.trim_end().chars().last();
    let str_end = msgstr.trim_end().chars().last();
    let id_punct = id_end.filter(|c| PUNCT.contains(c));
    let str_punct = str_end.filter(|c| PUNCT.contains(c));
    // Only fires when the source has trailing punctuation (P4)
    id_punct.is_some() && id_punct != str_punct
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
            flags: EntryFlags {
                fuzzy: false,
                obsolete: false,
            },
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
        assert!(
            has_code(&diags, "msg/fstring-in-call"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn req_diag_06_format_before_call_percent() {
        let c = calls(r#"_("Hi %s" % name)"#);
        let diags = check_source(&c, &empty_index());
        assert!(
            has_code(&diags, "msg/format-before-call"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn req_diag_06_format_before_call_method() {
        let c = calls(r#"_("Hello {}".format(name))"#);
        let diags = check_source(&c, &empty_index());
        assert!(
            has_code(&diags, "msg/format-before-call"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn req_diag_06_non_constant_id() {
        let c = calls(r#"_(label)"#);
        let diags = check_source(&c, &empty_index());
        assert!(
            has_code(&diags, "msg/non-constant-id"),
            "got: {:?}",
            codes(&diags)
        );
    }

    // ── REQ-DIAG-02: unresolved call → msgid checks stay silent ──────────────

    #[test]
    fn req_diag_02_fstring_skips_unknown_id() {
        let c = calls(r#"_(f"Hello {user}")"#);
        let diags = check_source(&c, &empty_index());
        assert!(
            !has_code(&diags, "msg/unknown-id"),
            "got: {:?}",
            codes(&diags)
        );
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
        assert!(
            has_code(&diags, "msg/implicit-concat"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn regular_string_does_not_fire_implicit_concat() {
        let index = CatalogIndex::build(vec![make_entry("de", "Hello", "Hallo")]);
        let c = calls(r#"_("Hello")"#);
        let diags = check_source(&c, &index);
        assert!(!has_code(&diags, "msg/implicit-concat"));
    }

    // ── is_in_pot includes msgctxt (babel-lsp-iyz) ───────────────────────────

    #[test]
    fn is_in_pot_includes_msgctxt_in_key() {
        // pgettext("menu", "File") must NOT suppress unknown-id for _("File")
        // and vice-versa — the msgctxt is part of the key.
        let mut pot = make_entry("", "File", "");
        pot.msgctxt = Some("menu".into());
        pot.file_path = "/locale/messages.pot".into();
        let index = CatalogIndex::build(vec![pot]);

        // _("File") has no context → key {msgid:"File", msgctxt:None} → not in pot
        let c = calls(r#"_("File")"#);
        let diags = check_source(&c, &index);
        assert!(
            has_code(&diags, "msg/unknown-id"),
            "plain _(\"File\") should fire unknown-id when only pgettext(\"menu\",\"File\") is in pot"
        );
    }

    // ── REQ-DIAG-05: msg/unknown-id ──────────────────────────────────────────

    #[test]
    fn req_diag_05_unknown_id_fires_when_not_in_any_catalog() {
        let c = calls(r#"_("Unknown")"#);
        let diags = check_source(&c, &empty_index());
        assert!(
            has_code(&diags, "msg/unknown-id"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn req_diag_05_unknown_id_silent_when_in_pot() {
        let mut pot = make_entry("", "Checkout", "");
        pot.file_path = "/locale/messages.pot".into();
        let index = CatalogIndex::build(vec![pot]);
        let c = calls(r#"_("Checkout")"#);
        let diags = check_source(&c, &index);
        assert!(
            !has_code(&diags, "msg/unknown-id"),
            "should be silent when key is in pot"
        );
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
        assert_eq!(
            d.range.start.character, 2,
            "squiggle should start at the literal"
        );
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
        assert!(
            has_code(&diags, "msg/missing-in-locale"),
            "got: {:?}",
            codes(&diags)
        );
        let msg = diags
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "msg/missing-in-locale"))
            .unwrap()
            .message
            .clone();
        assert!(
            msg.contains("fr"),
            "message should name the missing locale, got: {}",
            msg
        );
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

    // ── check_catalog helpers ────────────────────────────────────────────────

    fn po_entry(msgid: &str, msgstr: &str) -> CatalogEntry {
        CatalogEntry {
            locale: "de".into(),
            domain: "messages".into(),
            msgid: msgid.into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec![msgstr.into()],
            flags: EntryFlags {
                fuzzy: false,
                obsolete: false,
            },
            file_path: PathBuf::from("/locale/de/LC_MESSAGES/messages.po"),
            line: 5,
        }
    }

    fn header_entry() -> CatalogEntry {
        CatalogEntry {
            locale: "de".into(),
            domain: "messages".into(),
            msgid: "".into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec![
                "Content-Type: text/plain; charset=UTF-8\nPlural-Forms: nplurals=2; plural=(n != 1);\n".into()
            ],
            flags: EntryFlags { fuzzy: false, obsolete: false },
            file_path: PathBuf::from("/locale/de/LC_MESSAGES/messages.po"),
            line: 1,
        }
    }

    fn test_uri() -> Uri {
        Uri::from_file_path("/locale/de/LC_MESSAGES/messages.po").unwrap()
    }

    fn cat_check(entries: Vec<CatalogEntry>) -> Vec<Diagnostic> {
        cat_check_with_index(entries, empty_index())
    }

    fn cat_check_with_index(entries: Vec<CatalogEntry>, index: CatalogIndex) -> Vec<Diagnostic> {
        let refs: Vec<&CatalogEntry> = entries.iter().collect();
        check_catalog(&refs, &test_uri(), &index)
    }

    // ── po/missing-translation ───────────────────────────────────────────────

    #[test]
    fn missing_translation_fires_on_empty_msgstr() {
        let diags = cat_check(vec![header_entry(), po_entry("Checkout", "")]);
        assert!(
            has_code(&diags, "po/missing-translation"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn missing_translation_silent_when_translated() {
        let diags = cat_check(vec![header_entry(), po_entry("Checkout", "Kasse")]);
        assert!(!has_code(&diags, "po/missing-translation"));
    }

    #[test]
    fn missing_translation_silent_when_fuzzy() {
        let mut e = po_entry("Checkout", "");
        e.flags.fuzzy = true;
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/missing-translation"));
    }

    // ── po/fuzzy ─────────────────────────────────────────────────────────────

    #[test]
    fn fuzzy_fires_on_fuzzy_entry() {
        let mut e = po_entry("Checkout", "Kasse");
        e.flags.fuzzy = true;
        let diags = cat_check(vec![header_entry(), e]);
        assert!(has_code(&diags, "po/fuzzy"), "got: {:?}", codes(&diags));
    }

    #[test]
    fn fuzzy_silent_when_not_fuzzy() {
        let diags = cat_check(vec![header_entry(), po_entry("Checkout", "Kasse")]);
        assert!(!has_code(&diags, "po/fuzzy"));
    }

    // ── po/duplicate-id ───────────────────────────────────────────────────────

    #[test]
    fn duplicate_id_fires_on_second_occurrence() {
        let e1 = po_entry("Checkout", "Kasse");
        let mut e2 = po_entry("Checkout", "Kasse");
        e2.line = 10;
        let diags = cat_check(vec![header_entry(), e1, e2]);
        assert!(
            has_code(&diags, "po/duplicate-id"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn req_diag_04_duplicate_id_has_related_information() {
        let e1 = po_entry("Checkout", "Kasse");
        let mut e2 = po_entry("Checkout", "Kasse");
        e2.line = 10;
        let diags = cat_check(vec![header_entry(), e1, e2]);
        let d = diags
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "po/duplicate-id"))
            .expect("expected po/duplicate-id");
        assert!(
            d.related_information.is_some(),
            "must carry relatedInformation"
        );
        let rel = d.related_information.as_ref().unwrap();
        assert!(!rel.is_empty());
        assert!(
            rel[0].message.contains("line"),
            "message should name the line"
        );
    }

    #[test]
    fn duplicate_id_silent_when_unique_keys() {
        let diags = cat_check(vec![
            header_entry(),
            po_entry("Checkout", "Kasse"),
            po_entry("Login", "Anmelden"),
        ]);
        assert!(!has_code(&diags, "po/duplicate-id"));
    }

    // ── po/obsolete ──────────────────────────────────────────────────────────

    #[test]
    fn req_diag_09_obsolete_fires_when_absent_from_pot() {
        let mut e = po_entry("OldKey", "Alt");
        e.flags.obsolete = true;
        // A .pot with a DIFFERENT key proves the template is loaded but OldKey is gone
        let mut pot = make_entry("", "SomeOtherKey", "");
        pot.file_path = "/locale/messages.pot".into();
        let index = CatalogIndex::build(vec![pot]);
        let diags = cat_check_with_index(vec![e], index);
        assert!(has_code(&diags, "po/obsolete"), "got: {:?}", codes(&diags));
    }

    #[test]
    fn req_diag_09_obsolete_silent_when_in_pot() {
        let mut e = po_entry("OldKey", "Alt");
        e.flags.obsolete = true;
        let mut pot = make_entry("", "OldKey", "");
        pot.file_path = "/locale/messages.pot".into();
        let index = CatalogIndex::build(vec![pot]);
        let diags = cat_check_with_index(vec![e], index);
        assert!(!has_code(&diags, "po/obsolete"));
    }

    #[test]
    fn req_diag_09_obsolete_silent_when_no_pot() {
        // No .pot loaded at all → "gone from template" is unprovable → silent (P4, REQ-DIAG-09)
        let mut e = po_entry("OldKey", "Alt");
        e.flags.obsolete = true;
        let diags = cat_check_with_index(vec![e], empty_index());
        assert!(!has_code(&diags, "po/obsolete"));
    }

    // ── po/format-mismatch / po/extra-variable ───────────────────────────────

    #[test]
    fn req_diag_07_format_mismatch_printf_missing() {
        let e = po_entry("%(num)d items in cart", "Artikel im Warenkorb");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/format-mismatch"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn req_diag_07_format_mismatch_brace_missing() {
        let e = po_entry("Hello {name}", "Hallo");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/format-mismatch"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn req_diag_07_extra_variable_fires_on_extra_in_msgstr() {
        // msgstr has %(extra)s not present in msgid
        let e = po_entry("Hello", "Hallo %(extra)s");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/extra-variable"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn format_mismatch_silent_when_placeholders_match() {
        let e = po_entry("%(num)d items", "%(num)d Artikel");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/format-mismatch"));
        assert!(!has_code(&diags, "po/extra-variable"));
    }

    #[test]
    fn req_diag_07_percent_percent_is_not_a_specifier() {
        let e = po_entry("100%% off", "100%% Rabatt");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/format-mismatch"));
    }

    // ── po/plural-count ───────────────────────────────────────────────────────

    #[test]
    fn req_diag_08_plural_count_fires_when_count_wrong() {
        let header = CatalogEntry {
            locale: "fr".into(),
            domain: "messages".into(),
            msgid: "".into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec!["Content-Type: text/plain; charset=UTF-8\nPlural-Forms: nplurals=2; plural=(n>1);\n".into()],
            flags: EntryFlags { fuzzy: false, obsolete: false },
            file_path: PathBuf::from("/locale/de/LC_MESSAGES/messages.po"),
            line: 1,
        };
        let mut e = po_entry("%(n)d item", "%(n)d article");
        e.msgid_plural = Some("%(n)d items".into());
        e.msgstr = vec!["%(n)d article".into()]; // only 1 form, but nplurals=2
        let diags = cat_check(vec![header, e]);
        assert!(
            has_code(&diags, "po/plural-count"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn req_diag_08_plural_count_silent_when_no_header() {
        // No parseable Plural-Forms → check stays silent (P4)
        let mut e = po_entry("%(n)d item", "%(n)d Artikel");
        e.msgid_plural = Some("%(n)d items".into());
        e.msgstr = vec!["%(n)d Artikel".into()]; // only 1, but no header to compare against
        let diags = cat_check(vec![e]);
        assert!(!has_code(&diags, "po/plural-count"));
    }

    #[test]
    fn req_diag_08_plural_count_silent_when_wholly_untranslated() {
        // Wholly untranslated plural → po/missing-translation, not po/plural-count
        let mut e = po_entry("%(n)d item", "");
        e.msgid_plural = Some("%(n)d items".into());
        e.msgstr = vec!["".into()];
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/plural-count"));
        assert!(has_code(&diags, "po/missing-translation"));
    }

    // ── po/header-missing ─────────────────────────────────────────────────────

    #[test]
    fn header_missing_fires_when_no_header_entry() {
        let diags = cat_check(vec![po_entry("Checkout", "Kasse")]);
        assert!(
            has_code(&diags, "po/header-missing"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn header_missing_fires_when_no_charset() {
        let mut h = header_entry();
        h.msgstr = vec!["Content-Type: text/plain\n".into()]; // no charset=
        let diags = cat_check(vec![h, po_entry("Checkout", "Kasse")]);
        assert!(
            has_code(&diags, "po/header-missing"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn header_missing_fires_when_plural_catalog_lacks_plural_forms() {
        let mut h = header_entry();
        h.msgstr = vec!["Content-Type: text/plain; charset=UTF-8\n".into()]; // no Plural-Forms
        let mut e = po_entry("%(n)d item", "%(n)d Artikel");
        e.msgid_plural = Some("%(n)d items".into());
        e.msgstr = vec!["%(n)d Artikel".into(), "%(n)d Artikel".into()];
        let diags = cat_check(vec![h, e]);
        assert!(
            has_code(&diags, "po/header-missing"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn header_missing_silent_when_header_complete() {
        let diags = cat_check(vec![header_entry(), po_entry("Checkout", "Kasse")]);
        assert!(!has_code(&diags, "po/header-missing"));
    }

    // ── po/accelerator-mismatch ──────────────────────────────────────────────

    #[test]
    fn req_diag_13_accelerator_mismatch_fires_when_msgid_has_one_and_msgstr_has_zero() {
        let e = po_entry("&File", "Datei"); // msgid has &, msgstr doesn't
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/accelerator-mismatch"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn req_diag_13_accelerator_silent_when_msgid_has_multiple() {
        let e = po_entry("A&File &Edit", "Datei & Bearbeiten");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/accelerator-mismatch"));
    }

    #[test]
    fn accelerator_silent_when_msgid_has_none() {
        // Zero markers in source → no accelerator to preserve → silent (REQ-DIAG-13)
        let e = po_entry("Health and Safety", "Gesundheit und Sicherheit");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/accelerator-mismatch"));
    }

    // ── po/escape-mismatch ───────────────────────────────────────────────────

    #[test]
    fn escape_mismatch_fires_when_tab_differs() {
        let e = po_entry("a\\tb", "ab"); // msgid has \t, msgstr doesn't
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/escape-mismatch"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn escape_mismatch_does_not_double_count_newline() {
        // \n differs → po/newline-count fires, NOT po/escape-mismatch
        let e = po_entry("a\\nb", "ab");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/escape-mismatch"));
    }

    // ── po/newline-count ─────────────────────────────────────────────────────

    #[test]
    fn newline_count_fires_when_counts_differ() {
        let e = po_entry("line1\nline2", "Zeile1");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/newline-count"),
            "got: {:?}",
            codes(&diags)
        );
    }

    // ── po/whitespace-edges ──────────────────────────────────────────────────

    #[test]
    fn whitespace_edges_fires_on_different_leading() {
        let e = po_entry(" Hello", "Hallo"); // msgid has leading space, msgstr doesn't
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/whitespace-edges"),
            "got: {:?}",
            codes(&diags)
        );
    }

    // ── po/end-punctuation ───────────────────────────────────────────────────

    #[test]
    fn end_punctuation_fires_when_period_missing_in_msgstr() {
        let e = po_entry("Save your changes.", "Änderungen speichern");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/end-punctuation"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn end_punctuation_silent_when_msgid_has_no_trailing_punct() {
        let e = po_entry("Save changes", "Änderungen speichern");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/end-punctuation"));
    }

    // ── po/xml-tag-mismatch ──────────────────────────────────────────────────

    #[test]
    fn xml_tag_mismatch_fires_when_tag_dropped() {
        let e = po_entry("Click <b>here</b>", "Hier klicken");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/xml-tag-mismatch"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn xml_tag_mismatch_silent_when_tags_match() {
        let e = po_entry("Click <b>here</b>", "Klick <b>hier</b>");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/xml-tag-mismatch"));
    }

    // ── po/unchanged ─────────────────────────────────────────────────────────

    #[test]
    fn req_diag_10_unchanged_fires_when_msgstr_eq_msgid() {
        let e = po_entry("Checkout", "Checkout"); // same, with lowercase
        let diags = cat_check(vec![header_entry(), e]);
        assert!(has_code(&diags, "po/unchanged"), "got: {:?}", codes(&diags));
    }

    #[test]
    fn req_diag_10_unchanged_silent_for_all_caps() {
        let e = po_entry("HTTP", "HTTP"); // no lowercase → legitimately identical
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/unchanged"));
    }

    // ── po/blank ─────────────────────────────────────────────────────────────

    #[test]
    fn blank_fires_when_msgstr_is_whitespace_only() {
        let e = po_entry("Checkout", "   ");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(has_code(&diags, "po/blank"), "got: {:?}", codes(&diags));
    }

    // ── po/bracket-count ─────────────────────────────────────────────────────

    #[test]
    fn bracket_count_fires_when_parens_differ() {
        let e = po_entry("Price (incl. tax)", "Preis exkl. Steuer");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/bracket-count"),
            "got: {:?}",
            codes(&diags)
        );
    }

    // ── po/double-space ───────────────────────────────────────────────────────

    #[test]
    fn double_space_fires_in_msgstr_but_not_msgid() {
        let e = po_entry("Hello world", "Hallo  Welt");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/double-space"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn double_space_silent_when_msgid_also_has_double_space() {
        let e = po_entry("Hello  world", "Hallo  Welt");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/double-space"));
    }

    // ── po/repeated-word ─────────────────────────────────────────────────────

    #[test]
    fn repeated_word_fires_in_msgstr() {
        let e = po_entry("the book", "das das Buch");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/repeated-word"),
            "got: {:?}",
            codes(&diags)
        );
    }

    // ── po/url-changed ───────────────────────────────────────────────────────

    #[test]
    fn url_changed_fires_when_url_dropped() {
        let e = po_entry("See https://example.com/path for details", "Siehe Details");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/url-changed"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn url_changed_silent_when_tld_swap_only() {
        let e = po_entry(
            "See https://example.com/docs",
            "Siehe https://example.de/docs",
        );
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/url-changed"));
    }

    // ── po/url-changed ───────────────────────────────────────────────────────

    #[test]
    fn url_changed_silent_when_trailing_period_differs_only() {
        // URL in prose ending with "." — the period is punctuation, not part of the URL
        let e = po_entry(
            "See https://example.com/path. For details.",
            "Siehe https://example.com/path. Für Details.",
        );
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            !has_code(&diags, "po/url-changed"),
            "trailing period on URL must not cause false positive, got: {:?}",
            codes(&diags)
        );
    }

    // ── po/number-mismatch ───────────────────────────────────────────────────

    #[test]
    fn number_mismatch_fires_when_numbers_differ() {
        let e = po_entry("You have 5 items", "Sie haben 3 Artikel");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/number-mismatch"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn number_mismatch_ignores_version_numbers_in_words() {
        // "UTF-8" and "UTF-8" contain "8" — but it's embedded in a word
        let e = po_entry("Use UTF-8 encoding", "Verwende UTF-8 Kodierung");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/number-mismatch"));
    }

    #[test]
    fn number_mismatch_silent_for_thousand_separator_formatting() {
        // "1,000" (English) vs "1000" (no separator) represent the same value
        let e = po_entry("You have 1,000 items", "Sie haben 1000 Artikel");
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            !has_code(&diags, "po/number-mismatch"),
            "thousand-separated 1,000 vs 1000 must not fire, got: {:?}",
            codes(&diags)
        );
    }

    // ── po/same-plurals ───────────────────────────────────────────────────────

    #[test]
    fn same_plurals_fires_when_all_forms_identical() {
        let mut e = po_entry("%(n)d item", "%(n)d Artikel");
        e.msgid_plural = Some("%(n)d items".into());
        e.msgstr = vec!["%(n)d Artikel".into(), "%(n)d Artikel".into()];
        let diags = cat_check(vec![header_entry(), e]);
        assert!(
            has_code(&diags, "po/same-plurals"),
            "got: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn same_plurals_silent_when_forms_differ() {
        let mut e = po_entry("%(n)d item", "%(n)d Artikel");
        e.msgid_plural = Some("%(n)d items".into());
        e.msgstr = vec!["%(n)d Artikel".into(), "%(n)d Artikel (pl)".into()];
        let diags = cat_check(vec![header_entry(), e]);
        assert!(!has_code(&diags, "po/same-plurals"));
    }

    // ── catalog severity contract ─────────────────────────────────────────────

    #[test]
    fn severity_missing_translation_is_information() {
        let diags = cat_check(vec![header_entry(), po_entry("Checkout", "")]);
        let d = diags.iter().find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "po/missing-translation")).unwrap();
        assert_eq!(d.severity, Some(DiagnosticSeverity::INFORMATION));
    }

    #[test]
    fn severity_fuzzy_is_information() {
        let mut e = po_entry("Checkout", "Kasse");
        e.flags.fuzzy = true;
        let diags = cat_check(vec![header_entry(), e]);
        let d = diags
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "po/fuzzy"))
            .unwrap();
        assert_eq!(d.severity, Some(DiagnosticSeverity::INFORMATION));
    }

    #[test]
    fn severity_duplicate_id_is_error() {
        let e1 = po_entry("Checkout", "Kasse");
        let mut e2 = po_entry("Checkout", "Kasse");
        e2.line = 10;
        let diags = cat_check(vec![header_entry(), e1, e2]);
        let d = diags
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "po/duplicate-id"))
            .unwrap();
        assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn severity_obsolete_is_hint() {
        let mut e = po_entry("OldKey", "Alt");
        e.flags.obsolete = true;
        let mut pot = make_entry("", "SomeOtherKey", "");
        pot.file_path = "/locale/messages.pot".into();
        let index = CatalogIndex::build(vec![pot]);
        let diags = cat_check_with_index(vec![e], index);
        let d = diags
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "po/obsolete"))
            .unwrap();
        assert_eq!(d.severity, Some(DiagnosticSeverity::HINT));
    }

    #[test]
    fn severity_format_mismatch_is_warning() {
        let e = po_entry("%(num)d items", "Artikel");
        let diags = cat_check(vec![header_entry(), e]);
        let d = diags
            .iter()
            .find(
                |d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "po/format-mismatch"),
            )
            .unwrap();
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn severity_extra_variable_is_warning() {
        let e = po_entry("Hello", "Hallo %(extra)s");
        let diags = cat_check(vec![header_entry(), e]);
        let d = diags
            .iter()
            .find(
                |d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "po/extra-variable"),
            )
            .unwrap();
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn severity_header_missing_is_warning() {
        let diags = cat_check(vec![po_entry("Checkout", "Kasse")]);
        let d = diags
            .iter()
            .find(
                |d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "po/header-missing"),
            )
            .unwrap();
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn severity_unchanged_is_hint() {
        let e = po_entry("Checkout", "Checkout");
        let diags = cat_check(vec![header_entry(), e]);
        let d = diags
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "po/unchanged"))
            .unwrap();
        assert_eq!(d.severity, Some(DiagnosticSeverity::HINT));
    }

    #[test]
    fn catalog_all_diagnostics_have_babel_lsp_source() {
        let e = po_entry("%(n)d items", "Artikel");
        let diags = cat_check(vec![header_entry(), e]);
        for d in &diags {
            assert_eq!(d.source.as_deref(), Some("babel-lsp"));
        }
    }

    // ── apply_diag_filter ─────────────────────────────────────────────────────

    use crate::config::DiagnosticsConfig;

    fn fuzzy_diag() -> Diagnostic {
        Diagnostic {
            range: Range::default(),
            severity: Some(DiagnosticSeverity::WARNING),
            code: Some(NumberOrString::String("po/fuzzy".into())),
            code_description: None,
            source: Some("babel-lsp".into()),
            message: "fuzzy".into(),
            related_information: None,
            tags: None,
            data: None,
        }
    }

    fn missing_diag() -> Diagnostic {
        Diagnostic {
            code: Some(NumberOrString::String("po/missing-translation".into())),
            message: "missing".into(),
            ..fuzzy_diag()
        }
    }

    #[test]
    fn filter_select_all_passes_everything() {
        let cfg = DiagnosticsConfig {
            select: vec!["all".into()],
            ..DiagnosticsConfig::default()
        };
        let out = apply_diag_filter(vec![fuzzy_diag(), missing_diag()], &cfg);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn filter_select_specific_passes_only_listed() {
        let cfg = DiagnosticsConfig {
            select: vec!["po/fuzzy".into()],
            ignore: vec![],
            severity: Default::default(),
        };
        let out = apply_diag_filter(vec![fuzzy_diag(), missing_diag()], &cfg);
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0].code, Some(NumberOrString::String(s)) if s == "po/fuzzy"));
    }

    #[test]
    fn filter_ignore_drops_listed_code() {
        let cfg = DiagnosticsConfig {
            select: vec!["all".into()],
            ignore: vec!["po/fuzzy".into()],
            severity: Default::default(),
        };
        let out = apply_diag_filter(vec![fuzzy_diag(), missing_diag()], &cfg);
        assert_eq!(out.len(), 1);
        assert!(
            matches!(&out[0].code, Some(NumberOrString::String(s)) if s == "po/missing-translation")
        );
    }

    #[test]
    fn filter_severity_override_changes_level() {
        let mut sev = std::collections::HashMap::new();
        sev.insert("po/fuzzy".into(), "error".into());
        let cfg = DiagnosticsConfig {
            select: vec!["all".into()],
            ignore: vec![],
            severity: sev,
        };
        let out = apply_diag_filter(vec![fuzzy_diag()], &cfg);
        assert_eq!(out[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn filter_severity_invalid_string_clears_level() {
        let mut sev = std::collections::HashMap::new();
        sev.insert("po/fuzzy".into(), "bogus".into());
        let cfg = DiagnosticsConfig {
            select: vec!["all".into()],
            ignore: vec![],
            severity: sev,
        };
        let out = apply_diag_filter(vec![fuzzy_diag()], &cfg);
        assert_eq!(out[0].severity, None);
    }

    #[test]
    fn filter_empty_input_returns_empty() {
        let out = apply_diag_filter(vec![], &DiagnosticsConfig::default());
        assert!(out.is_empty());
    }

    // ── check_project helpers ─────────────────────────────────────────────────

    use crate::extract::types::TranslationCall;

    fn po_entry2(
        locale: &str,
        domain: &str,
        file: &str,
        msgid: &str,
        msgstr: &str,
    ) -> CatalogEntry {
        CatalogEntry {
            locale: locale.into(),
            domain: domain.into(),
            msgid: msgid.into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec![msgstr.into()],
            flags: EntryFlags {
                fuzzy: false,
                obsolete: false,
            },
            file_path: PathBuf::from(file),
            line: 5,
        }
    }

    fn source_call(msgid: &str) -> TranslationCall {
        TranslationCall {
            func: TranslationFunc::Gettext,
            msgid: Some(msgid.into()),
            msgid_plural: None,
            msgctxt: None,
            domain: None,
            range: Range::default(),
            msgid_range: None,
            unresolved_reason: None,
            unresolved_arg_range: None,
            is_implicit_concat: false,
        }
    }

    fn unresolved_call() -> TranslationCall {
        TranslationCall {
            func: TranslationFunc::Gettext,
            msgid: None,
            msgid_plural: None,
            msgctxt: None,
            domain: None,
            range: Range::default(),
            msgid_range: None,
            unresolved_reason: Some(crate::extract::types::UnresolvedReason::NonConstant),
            unresolved_arg_range: None,
            is_implicit_concat: false,
        }
    }

    fn proj_check(
        entries: Vec<CatalogEntry>,
        calls: Vec<TranslationCall>,
    ) -> HashMap<Uri, Vec<Diagnostic>> {
        let index = CatalogIndex::build(entries);
        check_project(&index, &calls)
    }

    fn any_code(out: &HashMap<Uri, Vec<Diagnostic>>, code: &str) -> bool {
        out.values()
            .flatten()
            .any(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == code))
    }

    // ── proj/inconsistent-translation ────────────────────────────────────────

    #[test]
    fn inconsistent_fires_when_same_locale_has_divergent_msgstr() {
        let e1 = po_entry2(
            "de",
            "messages",
            "/locale/de/a/messages.po",
            "Checkout",
            "Kasse",
        );
        let e2 = po_entry2(
            "de",
            "messages",
            "/locale/de/b/messages.po",
            "Checkout",
            "Bezahlen",
        );
        let out = proj_check(vec![e1, e2], vec![]);
        assert!(
            any_code(&out, "proj/inconsistent-translation"),
            "got: {:?}",
            out
        );
    }

    #[test]
    fn inconsistent_carries_related_information() {
        let e1 = po_entry2(
            "de",
            "messages",
            "/locale/de/a/messages.po",
            "Checkout",
            "Kasse",
        );
        let e2 = po_entry2(
            "de",
            "messages",
            "/locale/de/b/messages.po",
            "Checkout",
            "Bezahlen",
        );
        let index = CatalogIndex::build(vec![e1, e2]);
        let out = check_project(&index, &[]);
        let diag = out.values().flatten()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(s)) if s == "proj/inconsistent-translation"))
            .expect("expected proj/inconsistent-translation");
        assert!(
            diag.related_information
                .as_ref()
                .map(|r| r.len() >= 2)
                .unwrap_or(false)
        );
    }

    #[test]
    fn inconsistent_silent_when_same_msgstr() {
        let e1 = po_entry2(
            "de",
            "messages",
            "/locale/de/a/messages.po",
            "Checkout",
            "Kasse",
        );
        let e2 = po_entry2(
            "de",
            "messages",
            "/locale/de/b/messages.po",
            "Checkout",
            "Kasse",
        );
        let out = proj_check(vec![e1, e2], vec![]);
        assert!(!any_code(&out, "proj/inconsistent-translation"));
    }

    #[test]
    fn inconsistent_silent_for_different_locales() {
        let de = po_entry2(
            "de",
            "messages",
            "/locale/de/messages.po",
            "Checkout",
            "Kasse",
        );
        let fr = po_entry2(
            "fr",
            "messages",
            "/locale/fr/messages.po",
            "Checkout",
            "Caisse",
        );
        let out = proj_check(vec![de, fr], vec![]);
        assert!(!any_code(&out, "proj/inconsistent-translation"));
    }

    #[test]
    fn inconsistent_skips_fuzzy_entries() {
        let e1 = po_entry2(
            "de",
            "messages",
            "/locale/de/a/messages.po",
            "Checkout",
            "Kasse",
        );
        let mut e2 = po_entry2(
            "de",
            "messages",
            "/locale/de/b/messages.po",
            "Checkout",
            "Bezahlen",
        );
        e2.flags.fuzzy = true;
        let out = proj_check(vec![e1, e2], vec![]);
        assert!(!any_code(&out, "proj/inconsistent-translation"));
    }

    // ── proj/unused-id ────────────────────────────────────────────────────────

    #[test]
    fn unused_id_fires_when_no_source_call_references_entry() {
        let e = po_entry2(
            "de",
            "messages",
            "/locale/de/messages.po",
            "Checkout",
            "Kasse",
        );
        let out = proj_check(vec![e], vec![]); // no calls at all
        assert!(any_code(&out, "proj/unused-id"), "got: {:?}", out);
    }

    #[test]
    fn unused_id_silent_when_call_references_id() {
        let e = po_entry2(
            "de",
            "messages",
            "/locale/de/messages.po",
            "Checkout",
            "Kasse",
        );
        let out = proj_check(vec![e], vec![source_call("Checkout")]);
        assert!(!any_code(&out, "proj/unused-id"));
    }

    #[test]
    fn unused_id_silent_p4_gate_on_unresolved_call() {
        let e = po_entry2(
            "de",
            "messages",
            "/locale/de/messages.po",
            "Checkout",
            "Kasse",
        );
        let out = proj_check(vec![e], vec![unresolved_call()]);
        assert!(!any_code(&out, "proj/unused-id"));
    }

    #[test]
    fn unused_id_fires_on_pot_entries() {
        let mut pot = po_entry2("", "messages", "/locale/messages.pot", "Checkout", "");
        pot.locale = "".into(); // pot entry has empty locale
        let index = CatalogIndex::build(vec![pot]);
        let out = check_project(&index, &[]);
        assert!(
            any_code(&out, "proj/unused-id"),
            "pot entry should fire unused-id, got: {:?}",
            out
        );
    }

    // ── proj/missing-locale-file ──────────────────────────────────────────────

    #[test]
    fn missing_locale_fires_when_locale_absent_from_domain() {
        let de = po_entry2(
            "de",
            "messages",
            "/locale/de/messages.po",
            "Checkout",
            "Kasse",
        );
        let fr = po_entry2("fr", "other", "/locale/fr/other.po", "Checkout", "Caisse");
        let out = proj_check(vec![de, fr], vec![]);
        // de has no "other" domain; fr has no "messages" domain
        assert!(any_code(&out, "proj/missing-locale-file"), "got: {:?}", out);
    }

    #[test]
    fn missing_locale_silent_when_all_locales_covered() {
        let de = po_entry2(
            "de",
            "messages",
            "/locale/de/messages.po",
            "Checkout",
            "Kasse",
        );
        let fr = po_entry2(
            "fr",
            "messages",
            "/locale/fr/messages.po",
            "Checkout",
            "Caisse",
        );
        let out = proj_check(vec![de, fr], vec![]);
        assert!(!any_code(&out, "proj/missing-locale-file"));
    }
}
