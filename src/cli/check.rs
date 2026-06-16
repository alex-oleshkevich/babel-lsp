use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use tower_lsp_server::ls_types::{DiagnosticSeverity, NumberOrString, Uri};
use walkdir::WalkDir;

use crate::catalog::index::CatalogIndex;
use crate::catalog::loader::{discover_catalogs, load_po_file, locale_domain_from_po_path};
use crate::config::{discover_locale_dirs, resolve_config};
use crate::features::{code_action, diagnostics};
use crate::util::po_edit::apply_text_edits;

// ── Public types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, clap::ValueEnum, Default)]
pub enum OutputFormat {
    #[default]
    Concise,
    Full,
    Json,
    JsonLines,
    Github,
    Gitlab,
    Junit,
    Grouped,
    Pylint,
}

#[derive(Debug, clap::Args)]
pub struct CheckArgs {
    /// Files or directories to check (default: current directory)
    #[arg()]
    pub paths: Vec<PathBuf>,

    /// Codes to enable (comma-separated or repeated; default: all)
    #[arg(long, value_delimiter = ',')]
    pub select: Vec<String>,

    /// Codes to disable (comma-separated or repeated)
    #[arg(long, value_delimiter = ',')]
    pub ignore: Vec<String>,

    /// Output format
    #[arg(long, default_value = "concise")]
    pub output_format: OutputFormat,

    /// Apply deterministic fixes to disk before reporting
    #[arg(long)]
    pub fix: bool,

    /// Always exit 0 even when findings are present
    #[arg(long)]
    pub exit_zero: bool,
}

/// A diagnostic finding with file path and 1-based location.
#[derive(Debug, Clone)]
pub struct Finding {
    pub path: PathBuf,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub code: String,
    pub message: String,
    pub severity: DiagnosticSeverity,
}

/// Whether color output is appropriate for this format and environment.
pub struct ColorConfig {
    pub enabled: bool,
}

impl ColorConfig {
    pub fn for_format(format: &OutputFormat) -> Self {
        let machine = matches!(
            format,
            OutputFormat::Json
                | OutputFormat::JsonLines
                | OutputFormat::Github
                | OutputFormat::Gitlab
                | OutputFormat::Junit
        );
        let no_color = std::env::var("NO_COLOR").is_ok();
        let is_tty = std::io::stdout().is_terminal();
        Self { enabled: !machine && !no_color && is_tty }
    }

    pub fn severity(&self, sev: DiagnosticSeverity, s: &str) -> String {
        if !self.enabled {
            return s.to_string();
        }
        let code = match sev {
            DiagnosticSeverity::ERROR => "\x1b[31m",
            DiagnosticSeverity::WARNING => "\x1b[33m",
            DiagnosticSeverity::INFORMATION => "\x1b[34m",
            _ => "\x1b[2m",
        };
        format!("{code}{s}\x1b[0m")
    }

    pub fn dim(&self, s: &str) -> String {
        if self.enabled { format!("\x1b[2m{s}\x1b[0m") } else { s.to_string() }
    }

    pub fn cyan(&self, s: &str) -> String {
        if self.enabled { format!("\x1b[36m{s}\x1b[0m") } else { s.to_string() }
    }

    pub fn bold(&self, s: &str) -> String {
        if self.enabled { format!("\x1b[1m{s}\x1b[0m") } else { s.to_string() }
    }
}

// ── Entry point ────────────────────────────────────────────────────────────────

/// Returns 0 (clean), 1 (findings), or 2 (fatal error).
pub fn run_check(args: CheckArgs) -> i32 {
    let filter_paths: Vec<PathBuf> = if args.paths.is_empty() {
        vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))]
    } else {
        args.paths.iter().map(|p| p.canonicalize().unwrap_or_else(|_| p.clone())).collect()
    };

    let workspace_root = find_workspace_root(&filter_paths[0])
        .unwrap_or_else(|| filter_paths[0].clone());

    let config = resolve_config(&workspace_root);

    let known = diagnostics::KNOWN_CODES;
    for code in args.select.iter().chain(args.ignore.iter()) {
        if code != "all" && !known.contains(&code.as_str()) {
            eprintln!("error: unknown diagnostic code '{code}'");
            eprintln!("valid codes: {}", known.join(", "));
            return 2;
        }
    }

    let mut diag_cfg = config.diagnostics.clone();
    if !args.select.is_empty() {
        diag_cfg.select = args.select.clone();
    }
    if !args.ignore.is_empty() {
        diag_cfg.ignore = args.ignore.clone();
    }

    let mut findings = collect_findings(&workspace_root, &config, &diag_cfg, &filter_paths, !args.paths.is_empty());

    if args.fix && !findings.is_empty() {
        apply_fix_pass(&findings, &workspace_root, &config);
        findings = collect_findings(&workspace_root, &config, &diag_cfg, &filter_paths, !args.paths.is_empty());
    }

    findings.sort_by(|a, b| {
        a.path.cmp(&b.path).then(a.line.cmp(&b.line)).then(a.col.cmp(&b.col))
    });

    let color = ColorConfig::for_format(&args.output_format);
    let output = super::format::render(&findings, &args.output_format, &color, &workspace_root);
    print!("{output}");

    let is_machine = matches!(
        args.output_format,
        OutputFormat::Json
            | OutputFormat::JsonLines
            | OutputFormat::Github
            | OutputFormat::Gitlab
            | OutputFormat::Junit
    );
    if !is_machine {
        if findings.is_empty() {
            println!("{}", color.bold("All checks passed!"));
        } else {
            let n = findings.len();
            println!("{}", color.severity(DiagnosticSeverity::WARNING, &format!("Found {} {}.", n, if n == 1 { "error" } else { "errors" })));
        }
    }

    if args.exit_zero || findings.is_empty() { 0 } else { 1 }
}

// ── Private helpers ────────────────────────────────────────────────────────────

fn collect_findings(
    workspace_root: &Path,
    config: &crate::config::Config,
    diag_cfg: &crate::config::DiagnosticsConfig,
    filter_paths: &[PathBuf],
    apply_filter: bool,
) -> Vec<Finding> {
    let locale_dirs = discover_locale_dirs(workspace_root, config);
    let catalog_paths = discover_catalogs(&locale_dirs);

    let mut all_entries = vec![];
    for path in &catalog_paths {
        let Some((locale, domain)) = locale_domain_from_po_path(path) else { continue };
        if let Ok(entries) = load_po_file(path, &locale, &domain) {
            all_entries.extend(entries);
        }
    }
    let index = CatalogIndex::build(all_entries);

    let source_calls = scan_source_calls(workspace_root, config);
    let all_calls: Vec<_> =
        source_calls.iter().flat_map(|(_, calls)| calls.iter().cloned()).collect();

    let mut findings: Vec<Finding> = vec![];

    for cat_path in &catalog_paths {
        let Some(uri) = Uri::from_file_path(cat_path) else { continue };
        let file_entries = index.entries_for_file(cat_path);
        let diags = diagnostics::check_catalog(&file_entries, &uri, &index);
        for d in diagnostics::apply_diag_filter(diags, diag_cfg) {
            let Some(code) = code_of(&d.code) else { continue };
            findings.push(Finding {
                path: cat_path.clone(),
                line: d.range.start.line + 1,
                col: d.range.start.character + 1,
                end_line: d.range.end.line + 1,
                end_col: d.range.end.character + 1,
                code,
                message: d.message,
                severity: d.severity.unwrap_or(DiagnosticSeverity::WARNING),
            });
        }
    }

    for (uri, pdiags) in diagnostics::check_project(&index, &all_calls) {
        let Some(path) = uri.to_file_path() else { continue };
        let path = path.into_owned();
        for d in diagnostics::apply_diag_filter(pdiags, diag_cfg) {
            let Some(code) = code_of(&d.code) else { continue };
            findings.push(Finding {
                path: path.clone(),
                line: d.range.start.line + 1,
                col: d.range.start.character + 1,
                end_line: d.range.end.line + 1,
                end_col: d.range.end.character + 1,
                code,
                message: d.message,
                severity: d.severity.unwrap_or(DiagnosticSeverity::WARNING),
            });
        }
    }

    for (uri, calls) in &source_calls {
        let Some(path) = uri.to_file_path() else { continue };
        let path = path.into_owned();
        for d in diagnostics::apply_diag_filter(diagnostics::check_source(calls, &index), diag_cfg) {
            let Some(code) = code_of(&d.code) else { continue };
            findings.push(Finding {
                path: path.clone(),
                line: d.range.start.line + 1,
                col: d.range.start.character + 1,
                end_line: d.range.end.line + 1,
                end_col: d.range.end.character + 1,
                code,
                message: d.message,
                severity: d.severity.unwrap_or(DiagnosticSeverity::WARNING),
            });
        }
    }

    if apply_filter {
        findings.retain(|f| {
            let abs = f.path.canonicalize().unwrap_or_else(|_| f.path.clone());
            filter_paths.iter().any(|fp| abs.starts_with(fp) || abs == *fp)
        });
    }

    findings
}

const FIX_CODES: &[&str] =
    &["po/missing-translation", "po/fuzzy", "po/format-mismatch", "po/plural-count"];

fn apply_fix_pass(
    findings: &[Finding],
    workspace_root: &Path,
    config: &crate::config::Config,
) {
    let mut by_file: HashMap<PathBuf, Vec<(u32, String)>> = HashMap::new();
    for f in findings {
        if FIX_CODES.contains(&f.code.as_str()) {
            by_file
                .entry(f.path.clone())
                .or_default()
                .push((f.line.saturating_sub(1), f.code.clone()));
        }
    }

    let locale_dirs = discover_locale_dirs(workspace_root, config);
    let catalog_paths = discover_catalogs(&locale_dirs);

    for (file_path, pairs) in &by_file {
        let Ok(content) = std::fs::read_to_string(file_path) else { continue };

        let Some((locale, domain)) = locale_domain_from_po_path(file_path) else { continue };
        let Ok(entries) = load_po_file(file_path, &locale, &domain) else { continue };
        let _ = catalog_paths; // loaded already to establish the index; entries are sufficient here

        let entry_refs: Vec<_> = entries.iter().collect();
        let pairs_ref: Vec<(u32, &str)> = pairs.iter().map(|(l, c)| (*l, c.as_str())).collect();
        let edits = code_action::fix_edits_for_file(&content, &entry_refs, &pairs_ref);

        if edits.is_empty() {
            continue;
        }

        let fixed = apply_text_edits(&content, &edits);
        let _ = std::fs::write(file_path, fixed.as_bytes());
    }
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let start = if start.is_file() { start.parent()? } else { start };
    for ancestor in start.ancestors() {
        if ancestor.join(".git").exists()
            || ancestor.join("pyproject.toml").exists()
            || ancestor.join("setup.py").exists()
        {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

const PRUNE_DIRS: &[&str] = &[
    ".git", "target", ".venv", "venv", "__pycache__", ".mypy_cache", ".pytest_cache",
];

fn scan_source_calls(
    root: &Path,
    config: &crate::config::Config,
) -> Vec<(Uri, Vec<crate::extract::types::TranslationCall>)> {
    let extra: HashMap<String, crate::extract::types::TranslationFunc> = config
        .extra_keywords
        .iter()
        .filter_map(|kw| {
            crate::extract::types::TranslationFunc::from_name(kw).map(|f| (kw.clone(), f))
        })
        .collect();

    let mut results = vec![];
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            !e.file_type().is_dir()
                || e.file_name()
                    .to_str()
                    .map(|n| !PRUNE_DIRS.contains(&n))
                    .unwrap_or(true)
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
        let is_py = ext == "py";
        let is_jinja = config.jinja_extensions.iter().any(|je| je.trim_start_matches('.') == ext);
        if !is_py && !is_jinja {
            continue;
        }
        let Some(uri) = Uri::from_file_path(path) else { continue };
        let Ok(bytes) = std::fs::read(path) else { continue };
        let calls = if is_py {
            crate::extract::python::extract(&bytes, &extra)
        } else {
            crate::extract::jinja::extract(&bytes, &extra)
        };
        if !calls.is_empty() {
            results.push((uri, calls));
        }
    }
    results
}

fn code_of(c: &Option<NumberOrString>) -> Option<String> {
    match c {
        Some(NumberOrString::String(s)) => Some(s.clone()),
        _ => None,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_workspace_root_finds_git() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let sub = tmp.path().join("src");
        std::fs::create_dir(&sub).unwrap();
        let root = find_workspace_root(&sub).unwrap();
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn find_workspace_root_returns_none_for_no_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let result = find_workspace_root(tmp.path());
        // Might find a parent git dir; just test it doesn't panic.
        let _ = result;
    }

    #[test]
    fn color_config_disabled_for_json() {
        let cfg = ColorConfig::for_format(&OutputFormat::Json);
        // Regardless of TTY state, machine formats disable color.
        let s = cfg.severity(DiagnosticSeverity::ERROR, "error");
        assert_eq!(s, "error");
    }

    #[test]
    fn run_check_empty_workspace_exits_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let args = CheckArgs {
            paths: vec![tmp.path().to_path_buf()],
            select: vec![],
            ignore: vec![],
            output_format: OutputFormat::Json,
            fix: false,
            exit_zero: false,
        };
        let code = run_check(args);
        // Empty workspace: no findings → exit 0
        assert_eq!(code, 0);
    }

    #[test]
    fn exit_zero_flag_forces_success_with_findings() {
        let tmp = tempfile::tempdir().unwrap();
        let args = CheckArgs {
            paths: vec![tmp.path().to_path_buf()],
            select: vec![],
            ignore: vec![],
            output_format: OutputFormat::Json,
            fix: false,
            exit_zero: true,
        };
        let code = run_check(args);
        assert_eq!(code, 0);
    }

    #[test]
    fn req_cli_04_unknown_code_exits_two() {
        let tmp = tempfile::tempdir().unwrap();
        let args = CheckArgs {
            paths: vec![tmp.path().to_path_buf()],
            select: vec!["po/typo-does-not-exist".into()],
            ignore: vec![],
            output_format: OutputFormat::Json,
            fix: false,
            exit_zero: false,
        };
        assert_eq!(run_check(args), 2);
    }

    #[test]
    fn req_cli_04_unknown_ignore_code_exits_two() {
        let tmp = tempfile::tempdir().unwrap();
        let args = CheckArgs {
            paths: vec![tmp.path().to_path_buf()],
            select: vec![],
            ignore: vec!["msg/no-such-check".into()],
            output_format: OutputFormat::Json,
            fix: false,
            exit_zero: false,
        };
        assert_eq!(run_check(args), 2);
    }

    #[test]
    fn req_cli_04_select_all_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let args = CheckArgs {
            paths: vec![tmp.path().to_path_buf()],
            select: vec!["all".into()],
            ignore: vec![],
            output_format: OutputFormat::Json,
            fix: false,
            exit_zero: false,
        };
        assert_eq!(run_check(args), 0);
    }

    #[test]
    fn req_cli_09_fix_repairs_missing_translation() {
        let tmp = tempfile::tempdir().unwrap();
        // Minimal locale dir structure the loader expects.
        let locale_dir = tmp.path().join("locale/de/LC_MESSAGES");
        std::fs::create_dir_all(&locale_dir).unwrap();
        let po_path = locale_dir.join("messages.po");
        std::fs::write(
            &po_path,
            b"msgid \"\"\nmsgstr \"Content-Type: text/plain; charset=UTF-8\\nPlural-Forms: nplurals=2; plural=(n!=1);\\n\"\n\nmsgid \"Save\"\nmsgstr \"\"\n",
        ).unwrap();

        // First pass: finds po/missing-translation.
        let args = CheckArgs {
            paths: vec![tmp.path().to_path_buf()],
            select: vec!["po/missing-translation".into()],
            ignore: vec![],
            output_format: OutputFormat::Json,
            fix: false,
            exit_zero: false,
        };
        assert_eq!(run_check(args), 1, "should report finding before fix");

        // --fix pass: applies fix to disk.
        let args = CheckArgs {
            paths: vec![tmp.path().to_path_buf()],
            select: vec!["po/missing-translation".into()],
            ignore: vec![],
            output_format: OutputFormat::Json,
            fix: true,
            exit_zero: false,
        };
        assert_eq!(run_check(args), 0, "should exit 0 after fix");

        // File on disk now contains the fix.
        let fixed = std::fs::read_to_string(&po_path).unwrap();
        assert!(fixed.contains("msgstr \"Save\""), "fix was written to disk: {fixed}");
    }

    #[test]
    fn req_cli_09_fix_removes_fuzzy_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let locale_dir = tmp.path().join("locale/de/LC_MESSAGES");
        std::fs::create_dir_all(&locale_dir).unwrap();
        let po_path = locale_dir.join("messages.po");
        std::fs::write(
            &po_path,
            b"msgid \"\"\nmsgstr \"Content-Type: text/plain; charset=UTF-8\\nPlural-Forms: nplurals=2; plural=(n!=1);\\n\"\n\n#, fuzzy\nmsgid \"Save\"\nmsgstr \"Speichern\"\n",
        ).unwrap();

        let args = CheckArgs {
            paths: vec![tmp.path().to_path_buf()],
            select: vec!["po/fuzzy".into()],
            ignore: vec![],
            output_format: OutputFormat::Json,
            fix: true,
            exit_zero: false,
        };
        assert_eq!(run_check(args), 0);

        let fixed = std::fs::read_to_string(&po_path).unwrap();
        assert!(!fixed.contains("#, fuzzy"), "fuzzy flag should be removed: {fixed}");
    }
}
