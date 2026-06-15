# Overview — babel-lsp

> **Status:** Draft
>
> **Version:** 0.1   ·   **Last updated:** 2026-06-15
>
> **Purpose:** What babel-lsp is, who it's for, and what it does — in plain language. Start here if you're new.
>
> **Related:** [roadmap](roadmap.md), [E01-architecture](foundations/E01-architecture.md)

---

## What it is

babel-lsp is a language server that understands Python [Babel](https://babel.pocoo.org/) internationalization the way a translation engineer does. It's a single Rust binary speaking the Language Server Protocol over stdio, so it works in any LSP-capable editor — Zed, Neovim, and Helix are the first-class targets.

A type checker sees `_("Checkout")` as a function call taking a string. babel-lsp sees a *message*: it knows which catalogs translate it, in which locales, whether any are fuzzy or missing, and whether the German translation kept the `%(num)d` placeholder intact.

## Who it's for

Developers and translators working on a Python or Jinja2 project that uses gettext-style i18n — the shopfront app from the constitution is the running example. They write `_(...)` calls in code, run `pybabel` to extract and update catalogs, and hand `.po` files to translators. babel-lsp lives in the seam between those activities.

## What it does

Each area below is a feature spec; this is the five-second version.

| Area | What you get | Spec |
|---|---|---|
| Catalog index | Every `.po`/`.pot` discovered, loaded, and indexed by message — the backbone every feature reads | [F01](features/F01-catalog-index.md) |
| Message extraction | Translation calls parsed from Python and Jinja with tree-sitter — all gettext variants, `{% trans %}` blocks | [F02](features/F02-message-extraction.md) |
| Diagnostics | Unknown msgids, missing/fuzzy/duplicate/obsolete entries, placeholder and plural mismatches, f-strings in `_()` | [F03](features/F03-diagnostics.md) |
| Completion | msgid autocomplete inside translation calls, with a multi-locale preview | [F04](features/F04-completion.md) |
| Hover | A translation table by locale, domain, and status over any msgid | [F05](features/F05-hover.md) |
| Navigation | Goto from a source msgid to its catalog entries; find every use; clickable `#:` source references | [F06](features/F06-navigation.md) |
| Code actions | Quick fixes on catalog entries (copy msgid, toggle fuzzy, fix placeholders, add plurals) | [F07](features/F07-code-actions.md) |
| Inlay hints | The translation in a chosen locale shown inline next to each call | [F08](features/F08-inlay-hints.md) |
| Symbols | Catalog entries as document symbols; any msgid searchable across the workspace | [F09](features/F09-symbols.md) |
| Rename | Rename a msgid across every catalog and call site at once | [F10](features/F10-rename.md) |
| Hardcoded strings | Find user-facing literals not wrapped in `_()`, and extract them into the catalog | [F11](features/F11-hardcoded-strings.md) |
| Code lens | "Used 3 times" and "2 of 3 locales translated" on calls and entries | [F12](features/F12-code-lens.md) |
| Catalog commands | Run extract / update / compile from the editor or the CLI | [F13](features/F13-catalog-commands.md) |
| Editor integration | Zed extension, Neovim and Helix config | [F14](features/F14-editor-integration.md) |
| CLI check mode | `babel-lsp check` runs the same diagnostics as a CI-friendly linter, ruff-style output | [F15](features/F15-cli.md) |
| Release & CI | GitHub Actions QA on every push, binaries on every tagged release | [F16](features/F16-release-ci.md) |

## What it isn't

- **Not a translation engine.** It never translates strings for you or calls a machine-translation API (per P1).
- **Not a replacement for `pybabel`.** It complements the Babel CLI; the catalog stays the source of truth (per P5). The catalog commands in [F13](features/F13-catalog-commands.md) wrap `pybabel`, they don't reimplement it.
- **Not a runtime tool.** It never imports your app or runs your views to discover messages — everything comes from reading source and catalog text (per P1).

## How it works, in one paragraph

The server scans the workspace, extracting per-file facts with tree-sitter: every translation call in your Python and Jinja, and every entry in your catalogs. A second, debounced pass builds the catalog index — a map from each `(msgid, msgctxt)` key to its entries across all locales and domains — and resolves each source msgid against it. Every LSP feature is then a pure lookup into that index. The full story is in [E01-architecture](foundations/E01-architecture.md).

## Cross-References

- **Related:** [roadmap](roadmap.md), [E01-architecture](foundations/E01-architecture.md), [index](index.md).

## Changelog

- **2026-06-15** — Initial overview: the sixteen spec areas, the shopfront framing, and the three non-goals.
