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
        let env = worktree.shell_env();

        // worktree.which uses the worktree's captured env; on a GUI-launched Zed
        // that env may lack PATH, so fall back to well-known install locations.
        let binary = worktree.which("babel-lsp").or_else(|| {
            let home = std::env::var("HOME").ok()?;
            let root = worktree.root_path();
            [
                format!("{root}/.venv/bin/babel-lsp"),
                format!("{root}/venv/bin/babel-lsp"),
                format!("{home}/.local/bin/babel-lsp"),
                format!("{home}/.cargo/bin/babel-lsp"),
            ]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
        })
        .ok_or_else(|| {
            "babel-lsp not found. Install with: pip install babel-lsp or cargo install babel-lsp"
                .to_string()
        })?;

        Ok(zed::Command {
            command: binary,
            args: vec!["lsp".into()],
            env,
        })
    }
}

zed::register_extension!(BabelExtension);

#[cfg(test)]
mod tests {
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
