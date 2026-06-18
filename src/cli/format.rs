use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use tower_lsp_server::ls_types::DiagnosticSeverity;

use super::check::{ColorConfig, Finding, OutputFormat};

pub fn render(
    findings: &[Finding],
    format: &OutputFormat,
    color: &ColorConfig,
    workspace_root: &Path,
) -> String {
    match format {
        OutputFormat::Concise => render_concise(findings, color, workspace_root),
        OutputFormat::Full => render_full(findings, color, workspace_root),
        OutputFormat::Json => render_json(findings, workspace_root),
        OutputFormat::JsonLines => render_json_lines(findings, workspace_root),
        OutputFormat::Github => render_github(findings, workspace_root),
        OutputFormat::Gitlab => render_gitlab(findings, workspace_root),
        OutputFormat::Junit => render_junit(findings, workspace_root),
        OutputFormat::Grouped => render_grouped(findings, color, workspace_root),
        OutputFormat::Pylint => render_pylint(findings, workspace_root),
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn display_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn sev_label(sev: DiagnosticSeverity) -> &'static str {
    match sev {
        DiagnosticSeverity::ERROR => "error",
        DiagnosticSeverity::WARNING => "warning",
        DiagnosticSeverity::INFORMATION => "info",
        _ => "hint",
    }
}

fn gitlab_severity(sev: DiagnosticSeverity) -> &'static str {
    match sev {
        DiagnosticSeverity::ERROR => "critical",
        DiagnosticSeverity::WARNING => "minor",
        _ => "info",
    }
}

fn fingerprint(path: &str, code: &str, line: u32, col: u32) -> String {
    // Stable deterministic fingerprint from path + code + location.
    let input = format!("{path}:{code}:{line}:{col}");
    let mut h: u64 = 0xcbf29ce484222325;
    for b in input.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn escape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn rule_url(code: &str) -> String {
    let slug = code.replace('/', "-");
    format!("https://babel-lsp.dev/rules/{slug}")
}

fn read_file_lines(path: &Path) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return vec![];
    };
    let mut s = String::new();
    let _ = f.read_to_string(&mut s);
    s.lines().map(str::to_owned).collect()
}

// ── Concise ────────────────────────────────────────────────────────────────────

fn render_concise(findings: &[Finding], color: &ColorConfig, root: &Path) -> String {
    let mut out = String::new();
    for f in findings {
        let path = display_path(&f.path, root);
        let code_msg = format!("{} {}", f.code, f.message);
        let colored = color.severity(f.severity, &code_msg);
        out.push_str(&format!("{}:{}:{}: {}\n", path, f.line, f.col, colored));
    }
    out
}

// ── Full ───────────────────────────────────────────────────────────────────────

fn render_full(findings: &[Finding], color: &ColorConfig, root: &Path) -> String {
    let mut file_cache: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    let mut out = String::new();

    for f in findings {
        let path_str = display_path(&f.path, root);
        let header = format!("{}: {}", f.code, f.message);
        out.push_str(&color.severity(f.severity, &header));
        out.push('\n');
        out.push_str(&format!(
            "  {} {}:{}:{}\n",
            color.blue("-->"),
            path_str,
            f.line,
            f.col
        ));
        out.push_str(&format!("   {}\n", color.blue("|")));

        let lines = file_cache
            .entry(f.path.clone())
            .or_insert_with(|| read_file_lines(&f.path));

        // Show context: one line before (if available) + the finding line.
        if f.line >= 2 {
            if let Some(ctx) = lines.get((f.line - 2) as usize) {
                out.push_str(&format!("{:>4} {} {}\n", f.line - 1, color.blue("|"), ctx));
            }
        }
        if let Some(src_line) = lines.get((f.line - 1) as usize) {
            out.push_str(&format!("{:>4} {} {}\n", f.line, color.blue("|"), src_line));
            // Use char count (not byte length) for correct caret alignment on
            // lines that contain multibyte UTF-8 characters.
            let col_chars = (f.col as usize).saturating_sub(1);
            let indent = " ".repeat(5 + col_chars);
            let width = if f.end_line == f.line {
                (f.end_col.saturating_sub(f.col) as usize).max(1)
            } else {
                src_line.chars().count().saturating_sub(col_chars).max(1)
            };
            let carets = "^".repeat(width);
            out.push_str(&format!(
                "   {} {}{}\n",
                color.blue("|"),
                indent,
                color.severity(f.severity, &carets)
            ));
        }
        out.push_str(&format!("   {}\n", color.blue("|")));
        out.push_str(&format!("{} {}\n", color.help_label("help:"), f.message));
        out.push('\n');
    }
    out
}

// ── JSON ───────────────────────────────────────────────────────────────────────

fn finding_to_json_obj(f: &Finding, root: &Path) -> String {
    let path = escape_json_str(&display_path(&f.path, root));
    let code = escape_json_str(&f.code);
    let message = escape_json_str(&f.message);
    let sev = sev_label(f.severity);
    let url = escape_json_str(&rule_url(&f.code));
    format!(
        r#"  {{
    "code": "{code}",
    "message": "{message}",
    "location": {{ "row": {}, "column": {} }},
    "end_location": {{ "row": {}, "column": {} }},
    "filename": "{path}",
    "severity": "{sev}",
    "url": "{url}",
    "fix": null
  }}"#,
        f.line, f.col, f.end_line, f.end_col
    )
}

fn render_json(findings: &[Finding], root: &Path) -> String {
    if findings.is_empty() {
        return "[]\n".to_string();
    }
    let items: Vec<_> = findings
        .iter()
        .map(|f| finding_to_json_obj(f, root))
        .collect();
    format!("[\n{}\n]\n", items.join(",\n"))
}

fn render_json_lines(findings: &[Finding], root: &Path) -> String {
    let mut out = String::new();
    for f in findings {
        let path = escape_json_str(&display_path(&f.path, root));
        let code = escape_json_str(&f.code);
        let message = escape_json_str(&f.message);
        let sev = sev_label(f.severity);
        let url = escape_json_str(&rule_url(&f.code));
        out.push_str(&format!(
            "{{\"code\":\"{code}\",\"message\":\"{message}\",\"location\":{{\"row\":{},\"column\":{}}},\"end_location\":{{\"row\":{},\"column\":{}}},\"filename\":\"{path}\",\"severity\":\"{sev}\",\"url\":\"{url}\",\"fix\":null}}\n",
            f.line, f.col, f.end_line, f.end_col
        ));
    }
    out
}

// ── GitHub ─────────────────────────────────────────────────────────────────────

fn render_github(findings: &[Finding], root: &Path) -> String {
    let mut out = String::new();
    for f in findings {
        let level = match f.severity {
            DiagnosticSeverity::ERROR => "error",
            DiagnosticSeverity::WARNING => "warning",
            _ => "notice",
        };
        let path = display_path(&f.path, root);
        let title = format!("babel-lsp ({})", f.code);
        let msg = f.message.replace('\n', "%0A");
        out.push_str(&format!(
            "::{level} title={title},file={path},line={},col={}::{msg}\n",
            f.line, f.col
        ));
    }
    out
}

// ── GitLab ─────────────────────────────────────────────────────────────────────

fn render_gitlab(findings: &[Finding], root: &Path) -> String {
    if findings.is_empty() {
        return "[]\n".to_string();
    }
    let mut items = String::new();
    for (i, f) in findings.iter().enumerate() {
        if i > 0 {
            items.push_str(",\n");
        }
        let path = escape_json_str(&display_path(&f.path, root));
        let code = escape_json_str(&f.code);
        let message = escape_json_str(&f.message);
        let sev = gitlab_severity(f.severity);
        let fp = fingerprint(&display_path(&f.path, root), &f.code, f.line, f.col);
        items.push_str(&format!(
            r#"  {{
    "check_name": "{code}",
    "description": "{message}",
    "severity": "{sev}",
    "fingerprint": "{fp}",
    "location": {{
      "path": "{path}",
      "lines": {{ "begin": {} }}
    }}
  }}"#,
            f.line
        ));
    }
    format!("[\n{items}\n]\n")
}

// ── JUnit ──────────────────────────────────────────────────────────────────────

fn render_junit(findings: &[Finding], root: &Path) -> String {
    let count = findings.len();
    let mut cases = String::new();
    for f in findings {
        let path = escape_xml(&display_path(&f.path, root));
        let code = escape_xml(&f.code);
        let message = escape_xml(&f.message);
        let body = escape_xml(&format!(
            "{}:{}:{}: {}: {}",
            display_path(&f.path, root),
            f.line,
            f.col,
            f.code,
            f.message
        ));
        cases.push_str(&format!(
            "    <testcase name=\"{code}\" classname=\"{path}\">\n      <failure message=\"{message}\">{body}</failure>\n    </testcase>\n"
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites>\n  <testsuite name=\"babel-lsp\" tests=\"{count}\" failures=\"{count}\" errors=\"0\">\n{cases}  </testsuite>\n</testsuites>\n"
    )
}

// ── Grouped ────────────────────────────────────────────────────────────────────

fn render_grouped(findings: &[Finding], color: &ColorConfig, root: &Path) -> String {
    let mut by_file: BTreeMap<PathBuf, Vec<&Finding>> = BTreeMap::new();
    for f in findings {
        by_file.entry(f.path.clone()).or_default().push(f);
    }
    let mut out = String::new();
    for (path, file_findings) in &by_file {
        let path_str = display_path(path, root);
        out.push_str(&format!("{}\n", color.bold(&path_str)));
        for f in file_findings {
            let loc = format!("  {}:{}", f.line, f.col);
            let code_msg = format!("{} {}", f.code, f.message);
            out.push_str(&format!(
                "{} {}\n",
                color.dim(&loc),
                color.severity(f.severity, &code_msg)
            ));
        }
        out.push('\n');
    }
    out
}

// ── Pylint ─────────────────────────────────────────────────────────────────────

fn render_pylint(findings: &[Finding], root: &Path) -> String {
    let mut out = String::new();
    for f in findings {
        let path = display_path(&f.path, root);
        out.push_str(&format!(
            "{}:{}: [{}] {}\n",
            path, f.line, f.code, f.message
        ));
    }
    out
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn finding(
        path: &str,
        line: u32,
        col: u32,
        code: &str,
        msg: &str,
        sev: DiagnosticSeverity,
    ) -> Finding {
        Finding {
            path: PathBuf::from(path),
            line,
            col,
            end_line: line,
            end_col: col + code.len() as u32,
            code: code.to_string(),
            message: msg.to_string(),
            severity: sev,
        }
    }

    fn root() -> PathBuf {
        PathBuf::from("/workspace")
    }

    fn no_color() -> ColorConfig {
        ColorConfig { enabled: false }
    }

    #[test]
    fn concise_format() {
        let f = finding(
            "/workspace/locale/de/messages.po",
            14,
            9,
            "po/format-mismatch",
            "placeholder missing",
            DiagnosticSeverity::WARNING,
        );
        let out = render_concise(&[f], &no_color(), &root());
        assert_eq!(
            out,
            "locale/de/messages.po:14:9: po/format-mismatch placeholder missing\n"
        );
    }

    #[test]
    fn pylint_format() {
        let f = finding(
            "/workspace/locale/de/messages.po",
            14,
            9,
            "po/format-mismatch",
            "placeholder missing",
            DiagnosticSeverity::WARNING,
        );
        let out = render_pylint(&[f], &root());
        assert_eq!(
            out,
            "locale/de/messages.po:14: [po/format-mismatch] placeholder missing\n"
        );
    }

    #[test]
    fn json_empty_is_array() {
        let out = render_json(&[], &root());
        assert_eq!(out, "[]\n");
    }

    #[test]
    fn json_has_required_fields() {
        let f = finding(
            "/workspace/locale/de/messages.po",
            14,
            9,
            "po/format-mismatch",
            "placeholder missing",
            DiagnosticSeverity::WARNING,
        );
        let out = render_json(&[f], &root());
        assert!(out.contains("\"code\": \"po/format-mismatch\""));
        assert!(out.contains("\"row\": 14"));
        assert!(out.contains("\"severity\": \"warning\""));
        assert!(out.contains("\"fix\": null"));
    }

    #[test]
    fn json_lines_one_object_per_line() {
        let f1 = finding(
            "/workspace/a.po",
            1,
            1,
            "po/fuzzy",
            "fuzzy",
            DiagnosticSeverity::INFORMATION,
        );
        let f2 = finding(
            "/workspace/b.po",
            2,
            1,
            "po/fuzzy",
            "fuzzy",
            DiagnosticSeverity::INFORMATION,
        );
        let out = render_json_lines(&[f1, f2], &root());
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with('{'));
        assert!(lines[0].ends_with('}'));
    }

    #[test]
    fn github_format_uses_workflow_commands() {
        let f = finding(
            "/workspace/locale/de/messages.po",
            14,
            9,
            "po/format-mismatch",
            "placeholder missing",
            DiagnosticSeverity::WARNING,
        );
        let out = render_github(&[f], &root());
        assert!(out.starts_with("::warning "));
        assert!(out.contains("file=locale/de/messages.po"));
        assert!(out.contains("line=14"));
    }

    #[test]
    fn github_error_uses_error_level() {
        let f = finding(
            "/workspace/a.po",
            1,
            1,
            "po/duplicate-id",
            "dup",
            DiagnosticSeverity::ERROR,
        );
        let out = render_github(&[f], &root());
        assert!(out.starts_with("::error "));
    }

    #[test]
    fn gitlab_empty_is_array() {
        let out = render_gitlab(&[], &root());
        assert_eq!(out, "[]\n");
    }

    #[test]
    fn gitlab_has_fingerprint_and_severity() {
        let f = finding(
            "/workspace/locale/de/messages.po",
            14,
            9,
            "po/format-mismatch",
            "placeholder missing",
            DiagnosticSeverity::WARNING,
        );
        let out = render_gitlab(&[f], &root());
        assert!(out.contains("\"check_name\": \"po/format-mismatch\""));
        assert!(out.contains("\"severity\": \"minor\""));
        assert!(out.contains("\"fingerprint\""));
    }

    #[test]
    fn junit_well_formed_xml() {
        let f = finding(
            "/workspace/locale/de/messages.po",
            14,
            9,
            "po/format-mismatch",
            "placeholder missing",
            DiagnosticSeverity::WARNING,
        );
        let out = render_junit(&[f], &root());
        assert!(out.contains("<?xml version=\"1.0\""));
        assert!(out.contains("<testsuites>"));
        assert!(out.contains("<failure"));
        assert!(out.contains("</testsuites>"));
    }

    #[test]
    fn junit_empty_workspace() {
        let out = render_junit(&[], &root());
        assert!(out.contains("tests=\"0\" failures=\"0\""));
    }

    #[test]
    fn grouped_format_groups_by_file() {
        let f1 = finding(
            "/workspace/a.po",
            1,
            1,
            "po/fuzzy",
            "fuzzy",
            DiagnosticSeverity::INFORMATION,
        );
        let f2 = finding(
            "/workspace/a.po",
            5,
            1,
            "po/missing-translation",
            "missing",
            DiagnosticSeverity::INFORMATION,
        );
        let f3 = finding(
            "/workspace/b.po",
            3,
            1,
            "po/fuzzy",
            "fuzzy",
            DiagnosticSeverity::INFORMATION,
        );
        let out = render_grouped(&[f1, f2, f3], &no_color(), &root());
        // a.po header appears before b.po header
        let a_pos = out.find("a.po").unwrap();
        let b_pos = out.find("b.po").unwrap();
        assert!(a_pos < b_pos);
    }

    #[test]
    fn display_path_strips_workspace_root() {
        let path = PathBuf::from("/workspace/locale/de/messages.po");
        let dp = display_path(&path, &root());
        assert_eq!(dp, "locale/de/messages.po");
    }

    #[test]
    fn display_path_keeps_absolute_when_outside_root() {
        let path = PathBuf::from("/other/locale/de/messages.po");
        let dp = display_path(&path, &root());
        assert_eq!(dp, "/other/locale/de/messages.po");
    }

    #[test]
    fn fingerprint_is_stable() {
        let a = fingerprint("locale/de/messages.po", "po/format-mismatch", 14, 9);
        let b = fingerprint("locale/de/messages.po", "po/format-mismatch", 14, 9);
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_differs_by_location() {
        let a = fingerprint("locale/de/messages.po", "po/format-mismatch", 14, 9);
        let b = fingerprint("locale/de/messages.po", "po/format-mismatch", 15, 9);
        assert_ne!(a, b);
    }
}
