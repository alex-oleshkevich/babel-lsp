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
        let binary = worktree
            .which("babel-lsp")
            .ok_or_else(|| "babel-lsp not found in PATH. Install with: cargo install babel-lsp".to_string())?;
        Ok(zed::Command {
            command: binary,
            args: vec!["lsp".into(), "--stdio".into()],
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(BabelExtension);

#[cfg(test)]
mod tests {
    use super::*;

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
