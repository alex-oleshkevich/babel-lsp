use zed_extension_api::{self as zed, settings::LspSettings, LanguageServerId, Result};

const SERVER_NAME: &str = "babel-lsp";

struct BabelExtension;

impl zed::Extension for BabelExtension {
    fn new() -> Self {
        BabelExtension
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let env = worktree.shell_env();

        if let Ok(lsp_settings) = LspSettings::for_worktree(SERVER_NAME, worktree) {
            if let Some(binary) = lsp_settings.binary {
                if let Some(path) = binary.path {
                    let args = binary.arguments.unwrap_or_else(|| vec!["lsp".into()]);
                    return Ok(zed::Command { command: path, args, env });
                }
            }
        }

        let binary = worktree
            .which(SERVER_NAME)
            .ok_or_else(|| format!("{SERVER_NAME} not found in PATH"))?;
        Ok(zed::Command {
            command: binary,
            args: vec!["lsp".into()],
            env,
        })
    }
}

zed::register_extension!(BabelExtension);
