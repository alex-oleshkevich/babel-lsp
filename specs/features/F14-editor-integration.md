# F14 — Editor Integration

> **Status:** Draft
>
> **Version:** 0.2   ·   **Last updated:** 2026-06-15
>
> **Purpose:** How babel-lsp ships to its first-class editors — a Zed extension, and Neovim and Helix configuration — plus the generic stdio path for any LSP client.
>
> **Depends on:** [E01-architecture](../foundations/E01-architecture.md), [constitution](../constitution.md)   ·   **Related:** [F13-catalog-commands](F13-catalog-commands.md), [F16-release-ci](F16-release-ci.md)

> Requirement tag: **EDIT**

---

## 1. Purpose & Scope

The server is editor-agnostic by construction (constitution P2); this spec covers the last mile per editor. It says how babel-lsp launches, which file types it attaches to, and which roots define a workspace.

Three editors are first-class targets: Zed, Neovim, and Helix. Each gets copy-pasteable config in this spec and the README. Every other LSP-capable editor reaches the server through the generic stdio path.

This spec covers:

- A minimal LSP-only Zed extension that ships in-repo.
- Neovim (`nvim-lspconfig`) and Helix (`languages.toml`) configuration snippets.
- The generic stdio launch any LSP client can use, including VS Code.
- The transport: stdio only in v1 (`--tcp`/`--http` deferred).
- The filetype and root-marker table every editor shares.

## 2. Non-Goals / Out of Scope

- A bespoke VS Code extension — not built in v1; VS Code rides the generic stdio path through a third-party LSP bridge.
- Editor-specific features. If a capability can't ship as standard LSP, it doesn't ship (P2).
- Packaging, registry publication, and the release workflow — owned by [F16-release-ci](F16-release-ci.md).
- How catalog commands are invoked and what they do — owned by [F13-catalog-commands](F13-catalog-commands.md); this spec only notes how each editor surfaces them.

## 3. Background & Rationale

babel-lsp links source translation calls to `.po`/`.pot` catalogs, so it must attach to two kinds of files at once: Python and Jinja source on one side, catalogs on the other. An editor that only attaches the server to `.py` files never sees the templates or the catalogs, and half the features go dark.

That makes the filetype list load-bearing, not cosmetic. Every snippet below names the source filetypes *and* the PO filetype, so the server attaches everywhere a fact lives.

The server speaks LSP over stdio and nothing else is required to run it (REQ-ARCH-01). The per-editor work is therefore configuration, not code — except Zed, which needs a tiny extension to register a third-party server at all.

## 4. Concepts & Definitions

- **First-class target** — an editor babel-lsp ships ready-to-use config for, tested each release: Zed, Neovim, Helix.
- **Generic stdio path** — launching `babel-lsp lsp --stdio` from any LSP client that can spawn a command. The lowest common denominator, available to every editor.
- **Root marker** — a file or directory whose presence marks a workspace root, so the server scans from the right folder. Canonical config lives in [E15](../foundations/E15-app-config.md).

## 5. Detailed Specification

### 5.1 Filetypes and roots

Every editor attaches the server to the same set of file types and resolves the workspace from the same markers. The list is the contract; the per-editor syntax below just expresses it.

| File type | Editor language name(s) | Why the server attaches |
|---|---|---|
| Python | `python` | Translation calls — `_()`, `ngettext()`, `pgettext()` ([F02](F02-message-extraction.md)). |
| Jinja | `jinja`, `htmldjango` | `{{ _(...) }}` and `{% trans %}` blocks in templates ([F02](F02-message-extraction.md)). |
| HTML | `html` | Templates often carry the `.html` extension; Jinja lives inside them. |
| PO / POT | `po` | Catalog entries — the source of truth ([F01](F01-catalog-index.md)). |

Workspace roots resolve from these markers, nearest first:

| Marker | Meaning |
|---|---|
| `pyproject.toml` | The Python project root; carries `[tool.babel-lsp]` config. |
| `.git` | Repository root, the fallback when no `pyproject.toml` is found. |

**REQ-EDIT-01 — One launch command everywhere.**

Every editor starts the server the same way: it runs `babel-lsp lsp --stdio`. The binary must be on `PATH`, or the editor must be given an absolute path to it. No editor passes a different subcommand or a different transport for normal use.

**REQ-EDIT-02 — Attach to source and catalog file types together.**

Each first-class editor config lists the Python, Jinja/HTML, *and* PO file types. A config that omits PO leaves catalog diagnostics and navigation dark; one that omits Jinja/HTML leaves template features dark. The full list in §5.1 is mandatory, not a menu.

**REQ-EDIT-03 — Coexist with the primary language server.**

babel-lsp runs *alongside* the user's Python LSP, never instead of it. Its diagnostics are namespaced (`source: "babel-lsp"`), and it claims no formatting or full-file ownership. In editors that need an opt-in to keep the default servers running, the config preserves them.

### 5.2 Zed (first-class, ships in-repo)

Zed is the only target needing code. Zed cannot launch a third-party language server from settings alone — a thin extension must register it. babel-lsp ships that extension under `editors/zed/`. It carries no grammar and no features; its whole job is to start the binary.

The manifest declares the extension and the languages the server attaches to. Python and PO are the two Zed built-in language names babel-lsp targets:

```toml
# editors/zed/extension.toml
id = "babel"
name = "babel"
version = "0.1.0"
schema_version = 1
authors = ["Babel LSP Contributors"]
description = "LSP-only Zed extension that starts babel-lsp"
repository = "https://github.com/your-org/babel-lsp"

[language_servers.babel_lsp]
languages = ["Python", "PO"]
```

The Rust glue implements one hook — `language_server_command` — returning the command Zed should spawn. It launches the binary from `PATH` with the stdio transport:

```rust
// editors/zed/src/lib.rs
fn language_server_command(
    &mut self,
    language_server_id: &zed::LanguageServerId,
    _worktree: &zed::Worktree,
) -> Result<zed::Command> {
    match language_server_id.as_ref() {
        "babel_lsp" | "babel-lsp" => Ok(zed::Command {
            command: "babel-lsp".to_string(),
            args: vec!["lsp".to_string(), "--stdio".to_string()],
            env: Default::default(),
        }),
        other => Err(format!("unsupported language server id: {other}").into()),
    }
}
```

**REQ-EDIT-04 — Zed extension is LSP-only and PATH-first.**

The extension declares no grammars and bundles no features. It locates `babel-lsp` on `PATH`. It registers for Zed's `Python` and `PO` languages so the server attaches in both `.py` and `.po` buffers.

Declaring a server in a Zed extension does not make Zed run it beside the default Python server. The user opts in by naming it in settings; the `"..."` entry keeps the built-in servers running:

```jsonc
// ~/.config/zed/settings.json
{
  "languages": {
    "Python": { "language_servers": ["babel_lsp", "..."] }
  }
}
```

Without this snippet the extension installs but the server never starts. The README shows it next to the install steps.

Zed has no command palette entry for arbitrary LSP commands, so the catalog commands of [F13](F13-catalog-commands.md) surface in Zed as **code actions** on the relevant range — the only LSP trigger Zed exposes for them.

### 5.3 Neovim

Neovim needs no plugin beyond `nvim-lspconfig`. The snippet sets the launch command, the full filetype list, and the root markers, then enables the server:

```lua
-- init.lua, using nvim-lspconfig
vim.lsp.config('babel_lsp', {
  cmd = { 'babel-lsp', 'lsp', '--stdio' },
  filetypes = { 'python', 'jinja', 'htmldjango', 'html', 'po' },
  root_markers = { 'pyproject.toml', '.git' },
})
vim.lsp.enable('babel_lsp')
```

The Jinja, HTML, and PO filetypes are load-bearing: drop them and the server is never attached to those buffers, so template and catalog features never fire (REQ-EDIT-02). The catalog commands of [F13](F13-catalog-commands.md) run from `:lua vim.lsp.buf.execute_command(...)` or appear as code actions via `vim.lsp.buf.code_action()`.

### 5.4 Helix

Helix configures the server in `languages.toml` and attaches it to each language alongside the user's existing servers:

```toml
# ~/.config/helix/languages.toml
[language-server.babel-lsp]
command = "babel-lsp"
args = ["lsp", "--stdio"]

[[language]]
name = "python"
language-servers = ["pyright", "babel-lsp"]

[[language]]
name = "jinja"
language-servers = ["babel-lsp"]

[[language]]
name = "po"
language-servers = ["babel-lsp"]
```

Order matters in Helix. It routes hover, goto-definition, and references to the *first* listed server that advertises the capability; only diagnostics, completion, code actions, and symbols merge across servers. With `pyright` first on `python`, its hover and goto stay primary, and babel-lsp's hover and string-goto are unavailable there — while diagnostics, completion, actions, and symbols still work. On `jinja` and `po`, babel-lsp is alone and primary. To make babel-lsp's hover and goto primary on Python, list it first and take the reverse trade. Catalog commands ([F13](F13-catalog-commands.md)) surface in Helix as code actions.

### 5.5 Generic stdio path (VS Code and any other client)

Any editor that can spawn a command and speak LSP can run babel-lsp through the generic path. There is no bespoke extension to install — the client launches the binary directly.

**REQ-EDIT-05 — The generic path is `babel-lsp lsp --stdio`.**

Any LSP client launches the server by spawning `babel-lsp lsp --stdio` and attaching it to the §5.1 file types with the §5.1 root markers. This is the contract every non-first-class editor uses.

VS Code reaches the server this way through a third-party generic LSP bridge extension (for example, a community "generic LSP client"), configured to launch the command above. babel-lsp ships no VS Code extension in v1; the generic bridge is the supported route.

### 5.6 Transports

stdio is the only transport v1 ships, and every editor config uses it.

**REQ-EDIT-06 — stdio is the only transport.**

`babel-lsp lsp --stdio` is the launch every editor uses (REQ-ARCH-01); `--stdio` is implied when no flag is given. v1 ships no remote transport — neither `--tcp` nor `--http` ([E01](../foundations/E01-architecture.md) resolves OQ-ARCH-2 to stdio-only). stdio reaches every first-class editor, so a socket transport is deferred until a concrete need appears.

## 6. Examples & Use Cases

A translator-engineer on the shopfront app uses Neovim. They drop the §5.3 snippet into `init.lua`. Opening `app/views.py`, the server attaches and flags a typo'd `_("Chekout")` no catalog knows. They open `app/templates/checkout.html` — because `html` and `jinja` are in `filetypes`, the server attaches there too and the `{{ _("Your cart") }}` call resolves. Opening `locale/de/LC_MESSAGES/messages.po`, the `po` filetype attaches the server again, and the placeholder check on the German `msgstr` runs. One binary, three file types, no plugin.

A teammate on Zed installs the in-repo extension, then adds the §5.2 settings opt-in so babel-lsp runs beside the default Python server. To run "update catalog from sources" ([F13](F13-catalog-commands.md)), they trigger it as a code action, since Zed exposes no command palette for LSP commands.

## 7. Edge Cases & Failure Modes

- Binary missing from `PATH` → each editor surfaces its own "server failed to start" error; the README troubleshooting section covers setting an absolute path.
- Zed extension installed but no settings opt-in → the server is registered but never started; the README flags this as the most common Zed mistake.
- PO filetype omitted from a config → catalog diagnostics and goto never appear, though source-side checks still run; the symptom is "diagnostics work in `.py` but not `.po`".
- Helix with the type checker listed first → babel-lsp hover and goto silently unavailable on Python; expected, documented in §5.4.
- Two servers fighting over diagnostics → cannot happen; babel-lsp namespaces its diagnostics with `source: "babel-lsp"` (REQ-EDIT-03).

## 8. Open Questions & Decisions

- **Decision (OQ-ARCH-2, owned by [E01](../foundations/E01-architecture.md))** — resolved to **stdio only** for v1; no `--tcp` or `--http`. Every launch command in this spec uses stdio.
- **Decision** — no bespoke VS Code extension in v1; VS Code uses the generic stdio bridge. Revisit if demand warrants a first-class extension.
- **Decision** — the Zed extension is LSP-only, locating the binary on `PATH`; a configurable binary path is a later enhancement, not v1.

## 9. Cross-References

- **Depends on:** [E01-architecture](../foundations/E01-architecture.md) — the stdio transport, REQ-ARCH-01; [constitution](../constitution.md) — P2 editor-agnostic.
- **Related:** [F13-catalog-commands](F13-catalog-commands.md) — how commands surface per editor; [F16-release-ci](F16-release-ci.md) — packaging and registry publication; [F01](F01-catalog-index.md)/[F02](F02-message-extraction.md) — the catalog and source facts editors attach to; [E15](../foundations/E15-app-config.md) — root markers and config.
- **Testing:** [E17 §2.5](../foundations/E17-testing.md) — the stdio `initialize` smoke test in the coverage matrix.

## 10. Changelog

- **2026-06-15** — v0.2: resolved OQ-ARCH-2 to **stdio only** — removed the `--tcp`/`--http` references from REQ-EDIT-06 and the launch commands.
- **2026-06-15** — Initial draft: first-class Zed extension (LSP-only, PATH-first), Neovim and Helix config snippets, the generic stdio path for VS Code and others, the shared filetype/root table, and the transport story.
