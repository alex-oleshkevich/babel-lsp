const RELEASE_YML: &str = include_str!("../.github/workflows/release.yml");
const PKG_SCRIPT: &str = include_str!("../scripts/package-zed-extension.sh");

// ── release.yml ───────────────────────────────────────────────────────────────

#[test]
fn release_yml_has_version_check_job() {
    assert!(
        RELEASE_YML.contains("version-check:"),
        "must have version-check job"
    );
}

#[test]
fn release_yml_version_check_compares_tag_to_cargo_toml() {
    assert!(
        RELEASE_YML.contains("GITHUB_REF_NAME"),
        "must read tag from GITHUB_REF_NAME"
    );
    assert!(
        RELEASE_YML.contains("cargo metadata"),
        "must read version from cargo metadata"
    );
}

#[test]
fn release_yml_build_matrix_has_five_targets() {
    let targets = [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ];
    for t in targets {
        assert!(RELEASE_YML.contains(t), "build matrix must include {t}");
    }
}

#[test]
fn release_yml_uses_cross_for_aarch64_linux() {
    assert!(
        RELEASE_YML.contains("cross: true"),
        "aarch64-linux must use cross"
    );
    assert!(
        RELEASE_YML.contains("cross build"),
        "must invoke cross for aarch64-linux"
    );
}

#[test]
fn release_yml_strips_binaries() {
    assert!(
        RELEASE_YML.contains("strip target/"),
        "must strip binaries before upload"
    );
}

#[test]
fn release_yml_packages_zed_extension() {
    assert!(
        RELEASE_YML.contains("package-zed-extension.sh"),
        "release job must package the Zed extension"
    );
}

#[test]
fn release_yml_updates_aur() {
    assert!(
        RELEASE_YML.contains("publish-aur:"),
        "must have AUR publish job"
    );
    assert!(
        RELEASE_YML.contains("aur.archlinux.org"),
        "must push to AUR"
    );
    assert!(
        RELEASE_YML.contains("AUR_SSH_KEY"),
        "must use AUR_SSH_KEY secret"
    );
}

#[test]
fn release_yml_no_github_context_in_run_commands() {
    // Verify GitHub expressions are passed via env vars, not inlined in run: scripts.
    // Look for the known-unsafe pattern: ${{ github.ref_name }} or ${{ github.repository }}
    // appearing directly inside a multi-line run: value.
    // A simple heuristic: these expressions should only appear under `env:` keys.
    let lines: Vec<&str> = RELEASE_YML.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        // Skip env: assignment lines (these are safe)
        if trimmed.starts_with("GH_") || trimmed.contains(": ${{") {
            continue;
        }
        // Flag any run: script line (not an env assignment) that embeds a github context
        if (trimmed.contains("${{ github.repository }}")
            || trimmed.contains("${{ github.ref_name }}"))
            && !trimmed.ends_with(": ${{ github.repository }}")
            && !trimmed.ends_with(": ${{ github.ref_name }}")
        {
            panic!(
                "Line {}: unsafe github context inline in run: command:\n  {}",
                i + 1,
                line
            );
        }
    }
}

// ── package-zed-extension.sh ──────────────────────────────────────────────────

#[test]
fn zed_package_script_has_bash_shebang() {
    assert!(
        PKG_SCRIPT.starts_with("#!/usr/bin/env bash"),
        "script must use bash"
    );
}

#[test]
fn zed_package_script_targets_wasip2() {
    assert!(
        PKG_SCRIPT.contains("wasm32-wasip2"),
        "must build for wasm32-wasip2"
    );
}

#[test]
fn zed_package_script_packages_extension_toml() {
    assert!(
        PKG_SCRIPT.contains("extension.toml"),
        "must include extension.toml"
    );
}

#[test]
fn zed_package_script_packages_wasm_binary() {
    assert!(
        PKG_SCRIPT.contains("extension.wasm"),
        "must include extension.wasm in zip"
    );
}
