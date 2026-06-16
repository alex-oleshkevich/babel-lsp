const NEOVIM_LUA: &str = include_str!("../editors/neovim/babel_lsp.lua");
const HELIX_TOML: &str = include_str!("../editors/helix/languages.toml");

// ── Neovim ────────────────────────────────────────────────────────────────────

#[test]
fn neovim_config_includes_python_filetype() {
    assert!(NEOVIM_LUA.contains("'python'"), "filetypes must include python");
}

#[test]
fn neovim_config_includes_jinja_filetype() {
    assert!(NEOVIM_LUA.contains("'jinja'"), "filetypes must include jinja");
}

#[test]
fn neovim_config_includes_html_filetypes() {
    assert!(NEOVIM_LUA.contains("'htmldjango'") && NEOVIM_LUA.contains("'html'"),
        "filetypes must include html and htmldjango");
}

#[test]
fn neovim_config_includes_po_filetype() {
    assert!(NEOVIM_LUA.contains("'po'"), "filetypes must include po (catalog files)");
}

#[test]
fn neovim_config_uses_correct_root_markers() {
    assert!(NEOVIM_LUA.contains("'pyproject.toml'"), "root_markers must include pyproject.toml");
    assert!(NEOVIM_LUA.contains("'.git'"), "root_markers must include .git");
}

#[test]
fn neovim_config_uses_stdio_transport() {
    assert!(NEOVIM_LUA.contains("'babel-lsp'") && NEOVIM_LUA.contains("'--stdio'"),
        "cmd must use babel-lsp lsp --stdio");
}

// ── Helix ─────────────────────────────────────────────────────────────────────

fn helix_config() -> toml::Value {
    toml::from_str(HELIX_TOML).expect("editors/helix/languages.toml must be valid TOML")
}

#[test]
fn helix_config_defines_language_server() {
    let config = helix_config();
    let ls = &config["language-server"]["babel-lsp"];
    assert_eq!(ls["command"].as_str().unwrap(), "babel-lsp");
    let args: Vec<&str> = ls["args"].as_array().unwrap().iter()
        .filter_map(|v| v.as_str()).collect();
    assert_eq!(args, ["lsp", "--stdio"]);
}

#[test]
fn helix_config_attaches_python() {
    let servers = helix_servers("python");
    assert!(servers.iter().any(|s| s == "babel-lsp"), "python must list babel-lsp");
}

#[test]
fn helix_config_attaches_jinja() {
    let servers = helix_servers("jinja");
    assert!(servers.iter().any(|s| s == "babel-lsp"), "jinja must list babel-lsp");
}

#[test]
fn helix_config_attaches_po() {
    let servers = helix_servers("po");
    assert!(servers.iter().any(|s| s == "babel-lsp"), "po must list babel-lsp (catalog files)");
}

fn helix_servers(lang: &str) -> Vec<String> {
    let config = helix_config();
    let languages = config["language"].as_array().expect("language array");
    for entry in languages {
        if entry["name"].as_str() == Some(lang) {
            return entry["language-servers"].as_array()
                .expect("language-servers array")
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
        }
    }
    vec![]
}
