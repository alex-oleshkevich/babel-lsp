use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const BUILTIN_INDICATORS: &[&str] = &["gettext", "_(", "ngettext", "pgettext", "{% trans"];
const LOCALE_DIR_NAMES: &[&str] = &["locales", "locale", "translations"];

// ── Public resolved types ────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct DiagnosticsConfig {
    pub select: Vec<String>,
    pub ignore: Vec<String>,
    pub severity: HashMap<String, String>,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            select: vec!["all".to_string()],
            ignore: vec![],
            severity: HashMap::new(),
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct UnchangedConfig {
    pub ignore: Vec<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Config {
    pub locale_dirs: Vec<PathBuf>,
    pub default_locale: Option<String>,
    pub domains: Option<Vec<String>>,
    pub extra_keywords: Vec<String>,
    pub jinja_extensions: Vec<String>,
    pub detect_hardcoded_strings: bool,
    pub inlay_hint_locale: Option<String>,
    pub position_encoding: String,
    pub diagnostics: DiagnosticsConfig,
    pub unchanged: UnchangedConfig,
    pub pybabel_path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            locale_dirs: vec![],
            default_locale: None,
            domains: None,
            extra_keywords: vec![],
            jinja_extensions: vec![
                ".html".to_string(),
                ".jinja2".to_string(),
                ".j2".to_string(),
            ],
            detect_hardcoded_strings: false,
            inlay_hint_locale: None,
            position_encoding: "utf-8".to_string(),
            diagnostics: DiagnosticsConfig::default(),
            unchanged: UnchangedConfig::default(),
            pybabel_path: None,
        }
    }
}

impl Config {
    pub fn indicators(&self) -> Vec<String> {
        BUILTIN_INDICATORS
            .iter()
            .map(|s| s.to_string())
            .chain(self.extra_keywords.iter().cloned())
            .collect()
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Resolve config from the three sources (babel.cfg < babel-lsp.toml < pyproject.toml).
pub fn resolve_config(workspace_root: &Path) -> Config {
    let mut partial = PartialConfig::default();
    for overlay in [
        load_babel_cfg(workspace_root),
        load_babel_lsp_toml(workspace_root),
        load_pyproject_toml(workspace_root),
    ]
    .into_iter()
    .flatten()
    {
        merge(&mut partial, overlay);
    }
    partial.into_config()
}

/// Return locale directories: explicit `locale_dirs` if set, else auto-discovered candidates.
#[allow(dead_code)]
pub fn discover_locale_dirs(root: &Path, config: &Config) -> Vec<PathBuf> {
    if !config.locale_dirs.is_empty() {
        return config.locale_dirs.clone();
    }
    let mut found = vec![];
    for name in LOCALE_DIR_NAMES {
        let p = root.join(name);
        if p.is_dir() {
            found.push(p);
        }
    }
    // Inside Python packages (dirs with __init__.py)
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("__init__.py").exists() {
                for name in LOCALE_DIR_NAMES {
                    let p = path.join(name);
                    if p.is_dir() {
                        found.push(p);
                    }
                }
            }
        }
    }
    found
}

// ── Serde-target partial config ───────────────────────────────────────────────

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct PartialDiagnosticsConfig {
    select: Option<Vec<String>>,
    ignore: Vec<String>,
    severity: HashMap<String, String>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct PartialUnchangedConfig {
    ignore: Vec<String>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct PartialConfig {
    locale_dirs: Vec<PathBuf>,
    default_locale: Option<String>,
    domains: Option<Vec<String>>,
    extra_keywords: Vec<String>,
    jinja_extensions: Vec<String>,
    detect_hardcoded_strings: Option<bool>,
    inlay_hint_locale: Option<String>,
    position_encoding: Option<String>,
    diagnostics: PartialDiagnosticsConfig,
    unchanged: PartialUnchangedConfig,
    pybabel_path: Option<PathBuf>,
}

impl PartialConfig {
    fn into_config(self) -> Config {
        let defaults = Config::default();
        Config {
            locale_dirs: self.locale_dirs,
            default_locale: self.default_locale,
            domains: self.domains,
            extra_keywords: self.extra_keywords,
            jinja_extensions: if self.jinja_extensions.is_empty() {
                defaults.jinja_extensions
            } else {
                self.jinja_extensions
            },
            detect_hardcoded_strings: self.detect_hardcoded_strings.unwrap_or(false),
            inlay_hint_locale: self.inlay_hint_locale,
            position_encoding: self.position_encoding.unwrap_or(defaults.position_encoding),
            diagnostics: DiagnosticsConfig {
                select: self
                    .diagnostics
                    .select
                    .unwrap_or(defaults.diagnostics.select),
                ignore: self.diagnostics.ignore,
                severity: self.diagnostics.severity,
            },
            unchanged: UnchangedConfig {
                ignore: self.unchanged.ignore,
            },
            pybabel_path: self.pybabel_path,
        }
    }
}

/// Merge `overlay` into `base` — overlay wins for each key it defines.
fn merge(base: &mut PartialConfig, overlay: PartialConfig) {
    if !overlay.locale_dirs.is_empty() {
        base.locale_dirs = overlay.locale_dirs;
    }
    if overlay.default_locale.is_some() {
        base.default_locale = overlay.default_locale;
    }
    if overlay.domains.is_some() {
        base.domains = overlay.domains;
    }
    if !overlay.extra_keywords.is_empty() {
        base.extra_keywords = overlay.extra_keywords;
    }
    if !overlay.jinja_extensions.is_empty() {
        base.jinja_extensions = overlay.jinja_extensions;
    }
    if overlay.detect_hardcoded_strings.is_some() {
        base.detect_hardcoded_strings = overlay.detect_hardcoded_strings;
    }
    if overlay.inlay_hint_locale.is_some() {
        base.inlay_hint_locale = overlay.inlay_hint_locale;
    }
    if overlay.position_encoding.is_some() {
        base.position_encoding = overlay.position_encoding;
    }
    if overlay.diagnostics.select.is_some() {
        base.diagnostics.select = overlay.diagnostics.select;
    }
    base.diagnostics.ignore.extend(overlay.diagnostics.ignore);
    base.diagnostics
        .severity
        .extend(overlay.diagnostics.severity);
    base.unchanged.ignore.extend(overlay.unchanged.ignore);
    if overlay.pybabel_path.is_some() {
        base.pybabel_path = overlay.pybabel_path;
    }
}

// ── File loaders ──────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct PyprojectWrapper {
    tool: Option<PyprojectTool>,
}

#[derive(Deserialize, Default)]
struct PyprojectTool {
    #[serde(rename = "babel-lsp", default)]
    babel_lsp: Option<PartialConfig>,
}

fn load_pyproject_toml(root: &Path) -> Option<PartialConfig> {
    let content = std::fs::read_to_string(root.join("pyproject.toml")).ok()?;
    let parsed: PyprojectWrapper = toml::from_str(&content).ok()?;
    parsed.tool?.babel_lsp
}

fn load_babel_lsp_toml(root: &Path) -> Option<PartialConfig> {
    let content = std::fs::read_to_string(root.join("babel-lsp.toml")).ok()?;
    toml::from_str(&content).ok()
}

fn load_babel_cfg(root: &Path) -> Option<PartialConfig> {
    let content = std::fs::read_to_string(root.join("babel.cfg")).ok()?;
    parse_babel_cfg_section(&content)
}

/// Parse the `[babel-lsp]` INI section from `babel.cfg` content.
fn parse_babel_cfg_section(content: &str) -> Option<PartialConfig> {
    let mut in_section = false;
    let mut partial = PartialConfig::default();
    let mut found = false;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == "[babel-lsp]";
            continue;
        }
        if !in_section || line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        found = true;
        let k = k.trim();
        let v = v.trim().trim_matches('"');
        match k {
            "extra_keywords" => {
                partial.extra_keywords = v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "jinja_extensions" => {
                partial.jinja_extensions = v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "locale_dirs" => {
                partial.locale_dirs = v
                    .split(',')
                    .map(|s| PathBuf::from(s.trim()))
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect();
            }
            "default_locale" => {
                partial.default_locale = Some(v.to_string());
            }
            _ => {}
        }
    }

    found.then_some(partial)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, content: &str) {
        std::fs::write(dir.path().join(name), content).unwrap();
    }

    // --- Config::indicators ---

    #[test]
    fn indicators_includes_builtins() {
        let cfg = Config::default();
        let inds = cfg.indicators();
        assert!(inds.contains(&"_(".to_string()));
        assert!(inds.contains(&"gettext".to_string()));
        assert!(inds.contains(&"{% trans".to_string()));
    }

    #[test]
    fn indicators_includes_extra_keywords() {
        let cfg = Config {
            extra_keywords: vec!["lazy_gettext".to_string()],
            ..Config::default()
        };
        assert!(cfg.indicators().contains(&"lazy_gettext".to_string()));
    }

    // --- resolve_config ---

    #[test]
    fn resolve_no_config_files_returns_defaults() {
        let dir = TempDir::new().unwrap();
        let cfg = resolve_config(dir.path());
        assert_eq!(cfg.position_encoding, "utf-8");
        assert!(cfg.extra_keywords.is_empty());
        assert_eq!(cfg.diagnostics.select, vec!["all"]);
    }

    #[test]
    fn resolve_reads_pyproject_toml() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "pyproject.toml",
            "[tool.babel-lsp]\ndefault_locale = \"de\"\nextra_keywords = [\"_t\"]\n",
        );
        let cfg = resolve_config(dir.path());
        assert_eq!(cfg.default_locale.as_deref(), Some("de"));
        assert!(cfg.extra_keywords.contains(&"_t".to_string()));
    }

    #[test]
    fn resolve_reads_babel_lsp_toml() {
        let dir = TempDir::new().unwrap();
        write(&dir, "babel-lsp.toml", "default_locale = \"fr\"\n");
        let cfg = resolve_config(dir.path());
        assert_eq!(cfg.default_locale.as_deref(), Some("fr"));
    }

    #[test]
    fn resolve_pyproject_overrides_babel_lsp_toml() {
        let dir = TempDir::new().unwrap();
        write(&dir, "babel-lsp.toml", "default_locale = \"fr\"\n");
        write(
            &dir,
            "pyproject.toml",
            "[tool.babel-lsp]\ndefault_locale = \"de\"\n",
        );
        let cfg = resolve_config(dir.path());
        assert_eq!(cfg.default_locale.as_deref(), Some("de"));
    }

    #[test]
    fn resolve_reads_babel_cfg_section() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "babel.cfg",
            "[babel-lsp]\nextra_keywords = _t, lazy_gettext\n",
        );
        let cfg = resolve_config(dir.path());
        assert!(cfg.extra_keywords.contains(&"_t".to_string()));
        assert!(cfg.extra_keywords.contains(&"lazy_gettext".to_string()));
    }

    #[test]
    fn resolve_pyproject_overrides_babel_cfg() {
        let dir = TempDir::new().unwrap();
        write(&dir, "babel.cfg", "[babel-lsp]\nextra_keywords = _t\n");
        write(
            &dir,
            "pyproject.toml",
            "[tool.babel-lsp]\nextra_keywords = [\"lazy_gettext\"]\n",
        );
        let cfg = resolve_config(dir.path());
        assert!(cfg.extra_keywords.contains(&"lazy_gettext".to_string()));
        assert!(!cfg.extra_keywords.contains(&"_t".to_string()));
    }

    #[test]
    fn resolve_diagnostics_defaults_to_all() {
        let dir = TempDir::new().unwrap();
        let cfg = resolve_config(dir.path());
        assert_eq!(cfg.diagnostics.select, vec!["all"]);
        assert!(cfg.diagnostics.ignore.is_empty());
    }

    #[test]
    fn resolve_diagnostics_select_from_pyproject() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "pyproject.toml",
            "[tool.babel-lsp.diagnostics]\nselect = [\"po/fuzzy\", \"po/missing-translation\"]\n",
        );
        let cfg = resolve_config(dir.path());
        assert_eq!(
            cfg.diagnostics.select,
            vec!["po/fuzzy", "po/missing-translation"]
        );
    }

    #[test]
    fn resolve_jinja_extensions_defaults_when_not_set() {
        let dir = TempDir::new().unwrap();
        let cfg = resolve_config(dir.path());
        assert!(cfg.jinja_extensions.contains(&".html".to_string()));
        assert!(cfg.jinja_extensions.contains(&".jinja2".to_string()));
    }

    // --- discover_locale_dirs ---

    #[test]
    fn discover_uses_explicit_locale_dirs() {
        let dir = TempDir::new().unwrap();
        let explicit = vec![dir.path().join("custom_locale")];
        let cfg = Config {
            locale_dirs: explicit.clone(),
            ..Config::default()
        };
        assert_eq!(discover_locale_dirs(dir.path(), &cfg), explicit);
    }

    #[test]
    fn discover_finds_root_locale_dirs() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("locale")).unwrap();
        std::fs::create_dir(dir.path().join("translations")).unwrap();
        let found = discover_locale_dirs(dir.path(), &Config::default());
        assert!(found.iter().any(|p| p.ends_with("locale")));
        assert!(found.iter().any(|p| p.ends_with("translations")));
    }

    #[test]
    fn discover_finds_package_locale_dirs() {
        let dir = TempDir::new().unwrap();
        let pkg = dir.path().join("myapp");
        std::fs::create_dir(&pkg).unwrap();
        std::fs::write(pkg.join("__init__.py"), "").unwrap();
        std::fs::create_dir(pkg.join("locale")).unwrap();
        let found = discover_locale_dirs(dir.path(), &Config::default());
        assert!(found.iter().any(|p| p.ends_with("locale")));
    }

    #[test]
    fn discover_skips_nonexistent_dirs() {
        let dir = TempDir::new().unwrap();
        let found = discover_locale_dirs(dir.path(), &Config::default());
        assert!(found.is_empty());
    }

    // --- parse_babel_cfg_section ---

    #[test]
    fn babel_cfg_ignores_other_sections() {
        let content = "[extractors]\n**.py = python\n[babel-lsp]\nextra_keywords = _t\n";
        let partial = parse_babel_cfg_section(content).unwrap();
        assert_eq!(partial.extra_keywords, vec!["_t"]);
    }

    #[test]
    fn babel_cfg_no_section_returns_none() {
        let content = "[extractors]\n**.py = python\n";
        assert!(parse_babel_cfg_section(content).is_none());
    }
}
