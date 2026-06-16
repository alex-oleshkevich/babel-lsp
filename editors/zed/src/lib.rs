use zed_extension_api::{self as zed, Result};

struct BabelExtension;

impl zed::Extension for BabelExtension {
    fn new() -> Self {
        BabelExtension
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let home = std::env::var("HOME").ok();
        let candidates = build_candidates(&worktree.root_path(), home.as_deref());
        let binary = resolve_binary(worktree.which("babel-lsp"), &candidates)
            .ok_or_else(|| "babel-lsp not found. Install with: pip install babel-lsp".to_string())?;
        Ok(zed::Command {
            command: binary,
            args: vec!["lsp".into(), "--stdio".into()],
            env: worktree.shell_env(),
        })
    }
}

fn build_candidates(root_path: &str, home: Option<&str>) -> Vec<String> {
    let mut paths = vec![format!("{}/.venv/bin/babel-lsp", root_path)];
    if let Some(h) = home {
        paths.push(format!("{}/.local/bin/babel-lsp", h));
        paths.push(format!("{}/.cargo/bin/babel-lsp", h));
    }
    paths
}

fn resolve_binary(which_result: Option<String>, candidates: &[String]) -> Option<String> {
    which_result.or_else(|| {
        candidates
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .cloned()
    })
}

zed::register_extension!(BabelExtension);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_candidates_venv_is_first() {
        let candidates = build_candidates("/project", Some("/home/user"));
        assert_eq!(candidates[0], "/project/.venv/bin/babel-lsp");
    }

    #[test]
    fn build_candidates_includes_pip_user_path() {
        let candidates = build_candidates("/project", Some("/home/user"));
        assert!(candidates.contains(&"/home/user/.local/bin/babel-lsp".to_string()));
    }

    #[test]
    fn build_candidates_includes_cargo_path() {
        let candidates = build_candidates("/project", Some("/home/user"));
        assert!(candidates.contains(&"/home/user/.cargo/bin/babel-lsp".to_string()));
    }

    #[test]
    fn build_candidates_no_home_means_venv_only() {
        let candidates = build_candidates("/project", None);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0], "/project/.venv/bin/babel-lsp");
    }

    #[test]
    fn resolve_binary_prefers_which_over_candidates() {
        let result = resolve_binary(
            Some("/usr/bin/babel-lsp".to_string()),
            &["/other/babel-lsp".to_string()],
        );
        assert_eq!(result.as_deref(), Some("/usr/bin/babel-lsp"));
    }

    #[test]
    fn resolve_binary_returns_none_when_all_absent() {
        let result = resolve_binary(None, &["/nonexistent/babel-lsp".to_string()]);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_binary_returns_none_with_empty_candidates() {
        let result = resolve_binary(None, &[]);
        assert_eq!(result, None);
    }

    #[test]
    fn po_language_config_registers_po_and_pot_suffixes() {
        let content = include_str!("../languages/po/config.toml");
        let table: toml::Table = toml::from_str(content).expect("valid TOML");
        let suffixes = table["path_suffixes"]
            .as_array()
            .expect("path_suffixes is an array");
        let strs: Vec<&str> = suffixes.iter().filter_map(|v| v.as_str()).collect();
        assert!(strs.contains(&"po"), "must include .po suffix");
        assert!(strs.contains(&"pot"), "must include .pot suffix");
    }
}
