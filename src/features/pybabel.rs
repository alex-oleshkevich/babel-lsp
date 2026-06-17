use std::io;
use std::path::PathBuf;
use std::process::Command;

/// Command IDs registered at initialize (REQ-CMD-01).
pub const COMMANDS: &[&str] = &["babel-lsp.extract", "babel-lsp.update", "babel-lsp.compile"];

#[derive(Debug, Clone, Copy)]
pub enum PybabelOp {
    Extract,
    Update,
    Compile,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub pybabel_path: Option<PathBuf>,
    pub locale_dirs: Vec<PathBuf>,
    pub domains: Option<Vec<String>>,
    pub locale: Option<String>,
    pub workspace_root: PathBuf,
}

pub enum RunResult {
    Success,
    Failure { exit_code: i32, stderr: String },
    NotFound,
}

/// Build the argument list for `op` without spawning a process.
pub fn build_args(op: PybabelOp, opts: &RunOptions) -> Vec<String> {
    let locale_dir = opts
        .locale_dirs
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("locale"));
    let domain = opts
        .domains
        .as_ref()
        .and_then(|d| d.first())
        .map(|s| s.as_str())
        .unwrap_or("messages");

    match op {
        PybabelOp::Extract => {
            let pot = locale_dir.join(format!("{domain}.pot"));
            vec![
                "extract".into(),
                "-F".into(),
                "babel.cfg".into(),
                "-o".into(),
                pot.to_string_lossy().into_owned(),
                ".".into(),
            ]
        }
        PybabelOp::Update => {
            let pot = locale_dir.join(format!("{domain}.pot"));
            let mut args = vec![
                "update".into(),
                "-i".into(),
                pot.to_string_lossy().into_owned(),
                "-d".into(),
                locale_dir.to_string_lossy().into_owned(),
            ];
            if let Some(locale) = &opts.locale {
                args.extend(["-l".into(), locale.clone()]);
            }
            args
        }
        PybabelOp::Compile => {
            let mut args = vec![
                "compile".into(),
                "-d".into(),
                locale_dir.to_string_lossy().into_owned(),
            ];
            if let Some(locale) = &opts.locale {
                args.extend(["-l".into(), locale.clone()]);
            }
            args
        }
    }
}

/// Spawn pybabel and return the result.
pub fn run_pybabel(op: PybabelOp, opts: &RunOptions) -> RunResult {
    let bin = opts
        .pybabel_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("pybabel"));
    let args = build_args(op, opts);

    match Command::new(&bin)
        .args(&args)
        .current_dir(&opts.workspace_root)
        .output()
    {
        Ok(output) if output.status.success() => RunResult::Success,
        Ok(output) => RunResult::Failure {
            exit_code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => RunResult::NotFound,
        Err(e) => RunResult::Failure {
            exit_code: -1,
            stderr: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn base_opts(tmp: &TempDir) -> RunOptions {
        RunOptions {
            pybabel_path: None,
            locale_dirs: vec![tmp.path().join("locale")],
            domains: None,
            locale: None,
            workspace_root: tmp.path().to_path_buf(),
        }
    }

    #[test]
    fn req_cmd_01_commands_constant_has_three_ids() {
        assert_eq!(COMMANDS.len(), 3);
        assert!(COMMANDS.contains(&"babel-lsp.extract"));
        assert!(COMMANDS.contains(&"babel-lsp.update"));
        assert!(COMMANDS.contains(&"babel-lsp.compile"));
    }

    #[test]
    fn req_cmd_07_extract_args_include_babel_cfg_and_output_pot() {
        let tmp = TempDir::new().unwrap();
        let args = build_args(PybabelOp::Extract, &base_opts(&tmp));
        assert_eq!(args[0], "extract");
        assert!(args.contains(&"-F".to_string()));
        assert!(args.contains(&"babel.cfg".to_string()));
        assert!(args.contains(&"-o".to_string()));
        assert!(args.iter().any(|a| a.ends_with("messages.pot")));
        assert!(args.contains(&".".to_string()));
    }

    #[test]
    fn req_cmd_07_update_args_include_input_and_dir() {
        let tmp = TempDir::new().unwrap();
        let args = build_args(PybabelOp::Update, &base_opts(&tmp));
        assert_eq!(args[0], "update");
        assert!(args.contains(&"-i".to_string()));
        assert!(args.iter().any(|a| a.ends_with("messages.pot")));
        assert!(args.contains(&"-d".to_string()));
    }

    #[test]
    fn req_cmd_07_compile_args_include_dir() {
        let tmp = TempDir::new().unwrap();
        let args = build_args(PybabelOp::Compile, &base_opts(&tmp));
        assert_eq!(args[0], "compile");
        assert!(args.contains(&"-d".to_string()));
    }

    #[test]
    fn req_cmd_07_locale_flag_appended_for_update_and_compile() {
        let tmp = TempDir::new().unwrap();
        let mut o = base_opts(&tmp);
        o.locale = Some("fr".into());

        let update = build_args(PybabelOp::Update, &o);
        let idx = update
            .iter()
            .position(|a| a == "-l")
            .expect("-l missing in update");
        assert_eq!(update[idx + 1], "fr");

        let compile = build_args(PybabelOp::Compile, &o);
        let idx = compile
            .iter()
            .position(|a| a == "-l")
            .expect("-l missing in compile");
        assert_eq!(compile[idx + 1], "fr");
    }

    #[test]
    fn req_cmd_07_extract_has_no_locale_flag() {
        let tmp = TempDir::new().unwrap();
        let mut o = base_opts(&tmp);
        o.locale = Some("fr".into());
        let args = build_args(PybabelOp::Extract, &o);
        assert!(
            !args.contains(&"-l".to_string()),
            "extract must not pass -l"
        );
    }

    #[test]
    fn req_cmd_07_custom_domain_used_in_pot_path() {
        let tmp = TempDir::new().unwrap();
        let mut o = base_opts(&tmp);
        o.domains = Some(vec!["myapp".into()]);
        let extract = build_args(PybabelOp::Extract, &o);
        assert!(extract.iter().any(|a| a.contains("myapp.pot")));
        let update = build_args(PybabelOp::Update, &o);
        assert!(update.iter().any(|a| a.contains("myapp.pot")));
    }

    #[test]
    fn req_cmd_07_not_found_when_pybabel_path_missing() {
        let tmp = TempDir::new().unwrap();
        let mut o = base_opts(&tmp);
        o.pybabel_path = Some(tmp.path().join("nonexistent_pybabel"));
        assert!(matches!(
            run_pybabel(PybabelOp::Compile, &o),
            RunResult::NotFound
        ));
    }

    #[test]
    #[cfg(unix)]
    fn req_cmd_08_failure_captures_stderr_and_exit_code() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let fake = tmp.path().join("pybabel");
        std::fs::write(&fake, "#!/bin/sh\n>&2 echo 'bad cfg'\nexit 1\n").unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();

        let mut o = base_opts(&tmp);
        o.pybabel_path = Some(fake);
        match run_pybabel(PybabelOp::Compile, &o) {
            RunResult::Failure { exit_code, stderr } => {
                assert_eq!(exit_code, 1);
                assert!(stderr.contains("bad cfg"), "stderr was: {stderr}");
            }
            _ => panic!("expected Failure"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn req_cmd_09_success_on_zero_exit() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let fake = tmp.path().join("pybabel");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();

        let mut o = base_opts(&tmp);
        o.pybabel_path = Some(fake);
        assert!(matches!(
            run_pybabel(PybabelOp::Compile, &o),
            RunResult::Success
        ));
    }
}
