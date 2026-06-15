# E02 — Folder Structure

> **Status:** Draft
>
> **Version:** 0.1   ·   **Last updated:** 2026-06-15
>
> **Purpose:** Where code lives and which way dependencies point — the source layout every module follows.
>
> **Depends on:** [E01-architecture](E01-architecture.md)   ·   **Related:** [E03-tech-stack](E03-tech-stack.md), [E17-testing](E17-testing.md)

> Requirement tag: **DIR**

---

## 1. Purpose & Scope

This spec defines the crate's module layout and the layering rule between them. It mirrors the two-pass architecture: extraction, catalog handling, features, and the shared state each get their own home.

## 2. Detailed Specification

### 2.1 The layout

**REQ-DIR-01 — One crate, modules by responsibility.**

The server is a single binary crate. Modules group by what they do, not by which LSP method calls them:

```
src/
├── main.rs            # CLI entry: clap subcommands, transport selection, tracing setup
├── server.rs          # impl LanguageServer for Backend — every request/notification handler
├── state.rs           # WorkspaceState, DocumentState (E07)
├── config.rs          # Config loading + locale discovery (E15)
├── cli/               # check / extract / update / compile subcommands (F15)
│   └── check.rs       # the headless linter and its output formats
├── extract/           # source-side pass 1 (F02)
│   ├── python.rs      # tree-sitter-python → TranslationCall
│   ├── jinja.rs       # tree-sitter-jinja → TranslationCall
│   └── types.rs       # TranslationCall, TranslationFunc
├── catalog/           # catalog-side pass 1 + pass 2 (F01)
│   ├── loader.rs      # discover + parse .po/.pot via polib
│   ├── index.rs       # CatalogIndex, CatalogEntry, CatalogKey (E07)
│   └── diagnostics.rs # catalog-side checks (F03)
├── features/          # pure-function LSP capabilities (F04–F12)
│   ├── completion.rs  hover.rs  definition.rs  references.rs
│   ├── code_action.rs  inlay_hint.rs  document_symbol.rs
│   ├── rename.rs  code_lens.rs  document_link.rs
│   └── diagnostics.rs # source-side checks + Finding dispatch (F03)
└── util/              # offset/encoding, URI, format-string, plural, PO-edit helpers
```

### 2.2 The layering rule

**REQ-DIR-02 — Dependencies flow downward; features never call each other.**

`util` depends on nothing internal. `extract` and `catalog` depend on `util` and `state`. `features` depend on `state`, `catalog`, `extract`, and `util` — but never on each other: every feature is a pure function of the state ([E01 REQ-ARCH-06](E01-architecture.md)), so cross-feature calls would smuggle in hidden coupling. `server.rs` and `cli/` sit on top, wiring requests to features. When two features need the same logic, it moves down into `util` or `catalog`, not sideways.

### 2.3 Tests

**REQ-DIR-03 — Unit tests inline, e2e tests out of tree.**

Rust unit tests live in `#[cfg(test)]` modules beside the code they test. The end-to-end suite — a real LSP client driving the built binary against fixture workspaces — lives in `tests/e2e/` with its fixtures, owned by [E17](E17-testing.md).

## 3. Cross-References

- **Depends on:** [E01-architecture](E01-architecture.md) — the two-pass shape this layout mirrors.
- **Related:** [E03-tech-stack](E03-tech-stack.md) — the crates each module uses; [E17-testing](E17-testing.md) — the `tests/` tree.

## 4. Changelog

- **2026-06-15** — Initial draft: the `extract` / `catalog` / `features` / `util` layout, the downward-only layering rule, and the unit-vs-e2e split.
