use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::catalog::index::{CatalogIndex, CatalogKey};
use crate::catalog::loader::{discover_catalogs, load_po_file, locale_domain_from_po_path};
use crate::config::{discover_locale_dirs, resolve_config};

use super::check::find_workspace_root;

#[derive(Debug, Clone, clap::ValueEnum, Default)]
pub enum StatsFormat {
    #[default]
    Table,
    Json,
}

#[derive(Debug, clap::Args)]
pub struct StatsArgs {
    /// Files or directories to analyse (default: current directory)
    #[arg()]
    pub paths: Vec<PathBuf>,

    /// Output format
    #[arg(long, default_value = "table")]
    pub output_format: StatsFormat,
}

#[derive(Debug)]
struct LocaleStats {
    total: usize,
    translated: usize,
    fuzzy: usize,
    missing: usize,
}

impl LocaleStats {
    fn translated_pct(&self) -> u32 {
        if self.total == 0 {
            100
        } else {
            ((self.translated as f64 / self.total as f64) * 100.0).round() as u32
        }
    }
}

/// Returns 0 always (stats is a reporting command, never a gate).
pub fn run_stats(args: StatsArgs) -> i32 {
    let start = if args.paths.is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        args.paths[0].canonicalize().unwrap_or_else(|_| args.paths[0].clone())
    };

    let workspace_root = find_workspace_root(&start).unwrap_or(start);
    let config = resolve_config(&workspace_root);

    let locale_dirs = discover_locale_dirs(&workspace_root, &config);
    let catalog_paths = discover_catalogs(&locale_dirs);

    let mut all_entries = vec![];
    for path in &catalog_paths {
        let Some((locale, domain)) = locale_domain_from_po_path(path) else { continue };
        if let Ok(entries) = load_po_file(path, &locale, &domain) {
            all_entries.extend(entries);
        }
    }
    let index = CatalogIndex::build(all_entries);

    if index.all_locales().is_empty() {
        eprintln!("no catalogs found");
        match args.output_format {
            StatsFormat::Table => print!("{}", table_header()),
            StatsFormat::Json => println!("[]"),
        }
        return 0;
    }

    let source_keys: Vec<&CatalogKey> = if index.has_pot_entries() {
        index.all_pot_keys().filter(|k| !k.msgid.is_empty()).collect()
    } else {
        index.all_msgids().filter(|k| !k.msgid.is_empty()).collect()
    };
    let total = source_keys.len();

    let mut by_locale: BTreeMap<String, LocaleStats> = BTreeMap::new();
    for locale in index.all_locales() {
        let mut stats = LocaleStats { total, translated: 0, fuzzy: 0, missing: 0 };
        for key in &source_keys {
            let entries = index.lookup(key);
            let entry = entries.iter().find(|e| &e.locale == locale && !e.flags.obsolete);
            match entry {
                None => stats.missing += 1,
                Some(e) if e.flags.fuzzy => stats.fuzzy += 1,
                Some(e) if e.msgstr.iter().all(|s| s.is_empty()) => stats.missing += 1,
                Some(_) => stats.translated += 1,
            }
        }
        by_locale.insert(locale.clone(), stats);
    }

    match args.output_format {
        StatsFormat::Table => print!("{}", render_table(&by_locale)),
        StatsFormat::Json => print!("{}", render_json(&by_locale)),
    }

    0
}

fn table_header() -> &'static str {
    "Locale   Messages   Translated   Fuzzy   Missing\n"
}

fn render_table(by_locale: &BTreeMap<String, LocaleStats>) -> String {
    let mut out = table_header().to_string();
    for (locale, s) in by_locale {
        let translated_col = format!("{} ({}%)", s.translated, s.translated_pct());
        out.push_str(&format!(
            "{:<8} {:>8}   {:>10}   {:>5}   {:>7}\n",
            locale, s.total, translated_col, s.fuzzy, s.missing
        ));
    }
    out
}

fn render_json(by_locale: &BTreeMap<String, LocaleStats>) -> String {
    if by_locale.is_empty() {
        return "[]\n".to_string();
    }
    let items: Vec<String> = by_locale
        .iter()
        .map(|(locale, s)| {
            format!(
                r#"  {{"locale":"{locale}","total":{},"translated":{},"translated_pct":{},"fuzzy":{},"missing":{}}}"#,
                s.total, s.translated, s.translated_pct(), s.fuzzy, s.missing
            )
        })
        .collect();
    format!("[\n{}\n]\n", items.join(",\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_stats_empty_workspace_exits_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let args = StatsArgs {
            paths: vec![tmp.path().to_path_buf()],
            output_format: StatsFormat::Json,
        };
        assert_eq!(run_stats(args), 0);
    }

    #[test]
    fn run_stats_with_catalog_produces_table() {
        let tmp = tempfile::tempdir().unwrap();
        let locale_dir = tmp.path().join("locale/de/LC_MESSAGES");
        std::fs::create_dir_all(&locale_dir).unwrap();
        std::fs::write(
            locale_dir.join("messages.po"),
            b"msgid \"\"\nmsgstr \"Content-Type: text/plain; charset=UTF-8\\nPlural-Forms: nplurals=2; plural=(n!=1);\\n\"\n\nmsgid \"Save\"\nmsgstr \"Speichern\"\n\nmsgid \"Cancel\"\nmsgstr \"\"\n",
        ).unwrap();

        let args = StatsArgs {
            paths: vec![tmp.path().to_path_buf()],
            output_format: StatsFormat::Table,
        };
        assert_eq!(run_stats(args), 0);
    }

    #[test]
    fn translated_pct_correct() {
        let s = LocaleStats { total: 100, translated: 97, fuzzy: 2, missing: 1 };
        assert_eq!(s.translated_pct(), 97);
    }

    #[test]
    fn translated_pct_zero_total() {
        let s = LocaleStats { total: 0, translated: 0, fuzzy: 0, missing: 0 };
        assert_eq!(s.translated_pct(), 100);
    }

    #[test]
    fn render_json_empty() {
        let out = render_json(&BTreeMap::new());
        assert_eq!(out, "[]\n");
    }

    #[test]
    fn render_json_has_required_fields() {
        let mut m = BTreeMap::new();
        m.insert("de".into(), LocaleStats { total: 10, translated: 8, fuzzy: 1, missing: 1 });
        let out = render_json(&m);
        assert!(out.contains("\"locale\":\"de\""));
        assert!(out.contains("\"total\":10"));
        assert!(out.contains("\"translated\":8"));
        assert!(out.contains("\"translated_pct\":80"));
        assert!(out.contains("\"fuzzy\":1"));
        assert!(out.contains("\"missing\":1"));
    }

    #[test]
    fn render_table_contains_locale_and_counts() {
        let mut m = BTreeMap::new();
        m.insert("de".into(), LocaleStats { total: 142, translated: 138, fuzzy: 3, missing: 1 });
        let out = render_table(&m);
        assert!(out.contains("Locale"), "header missing");
        assert!(out.contains("de"), "locale missing");
        assert!(out.contains("142"), "total missing");
        assert!(out.contains("138"), "translated count missing");
        assert!(out.contains("97%"), "pct missing");
        assert!(out.contains("3"), "fuzzy missing");
    }

    #[test]
    fn render_table_sorted_by_locale() {
        let mut m = BTreeMap::new();
        m.insert("fr".into(), LocaleStats { total: 5, translated: 5, fuzzy: 0, missing: 0 });
        m.insert("de".into(), LocaleStats { total: 5, translated: 3, fuzzy: 1, missing: 1 });
        let out = render_table(&m);
        let de_pos = out.find("de").unwrap();
        let fr_pos = out.find("fr").unwrap();
        assert!(de_pos < fr_pos, "de should sort before fr");
    }
}
