use std::path::PathBuf;

use crate::cli::check::find_workspace_root;
use crate::config::{discover_locale_dirs, resolve_config};
use crate::features::pybabel::{PybabelOp, RunOptions, RunResult, run_pybabel};

#[derive(Debug, clap::Args)]
pub struct ExtractArgs {
    /// Workspace path (default: current directory)
    #[arg()]
    pub path: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct UpdateArgs {
    /// Workspace path (default: current directory)
    #[arg()]
    pub path: Option<PathBuf>,
    /// Limit to one locale
    #[arg(long, short)]
    pub locale: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct CompileArgs {
    /// Workspace path (default: current directory)
    #[arg()]
    pub path: Option<PathBuf>,
    /// Limit to one locale
    #[arg(long, short)]
    pub locale: Option<String>,
}

fn run_op(op: PybabelOp, path: Option<PathBuf>, locale: Option<String>) -> i32 {
    let start = path
        .map(|p| p.canonicalize().unwrap_or(p))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace_root = find_workspace_root(&start).unwrap_or(start);
    let config = resolve_config(&workspace_root);
    let locale_dirs = discover_locale_dirs(&workspace_root, &config);

    let opts = RunOptions {
        pybabel_path: config.pybabel_path,
        locale_dirs,
        domains: config.domains,
        locale,
        workspace_root,
    };

    match run_pybabel(op, &opts) {
        RunResult::Success => 0,
        RunResult::Failure { exit_code, stderr } => {
            eprintln!("pybabel error (exit {exit_code}):\n{stderr}");
            1
        }
        RunResult::NotFound => {
            eprintln!(
                "error: pybabel not found — install Babel or set `pybabel_path` in config"
            );
            1
        }
    }
}

pub fn run_extract(args: ExtractArgs) -> i32 {
    run_op(PybabelOp::Extract, args.path, None)
}

pub fn run_update(args: UpdateArgs) -> i32 {
    run_op(PybabelOp::Update, args.path, args.locale)
}

pub fn run_compile(args: CompileArgs) -> i32 {
    run_op(PybabelOp::Compile, args.path, args.locale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn req_cmd_04_cli_extract_exits_zero_with_fake_pybabel() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        let fake = tmp.path().join("pybabel");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();

        std::fs::write(
            tmp.path().join("babel-lsp.toml"),
            format!("pybabel_path = {:?}\n", fake.to_str().unwrap()),
        )
        .unwrap();

        let code = run_op(PybabelOp::Extract, Some(tmp.path().to_path_buf()), None);
        assert_eq!(code, 0);
    }

    #[test]
    #[cfg(unix)]
    fn req_cmd_04_cli_compile_with_locale_exits_zero() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        let fake = tmp.path().join("pybabel");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();

        std::fs::write(
            tmp.path().join("babel-lsp.toml"),
            format!("pybabel_path = {:?}\n", fake.to_str().unwrap()),
        )
        .unwrap();

        let code = run_op(
            PybabelOp::Compile,
            Some(tmp.path().to_path_buf()),
            Some("fr".into()),
        );
        assert_eq!(code, 0);
    }

    #[test]
    fn req_cmd_04_cli_exits_nonzero_when_pybabel_not_installed() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("babel-lsp.toml"),
            "pybabel_path = \"/nonexistent_pybabel_xyz\"\n",
        )
        .unwrap();

        let code = run_op(PybabelOp::Compile, Some(tmp.path().to_path_buf()), None);
        assert_ne!(code, 0);
    }
}
