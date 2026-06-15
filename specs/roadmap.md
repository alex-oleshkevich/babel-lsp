# Roadmap

> **Status:** Living (continuously maintained)
>
> **Last updated:** 2026-06-15
>
> **Purpose:** The build order — what ships in each milestone and why that sequence.
>
> **Related:** [01-overview](01-overview.md), [index](index.md)

---

## The shape of the build

The catalog index comes first because everything else reads from it: completion lists from it, hover renders it, diagnostics check against it, navigation jumps into it. After the index and extraction land, each milestone is independently shippable — the server is useful from M1 onward and simply gets smarter.

The first public release is **M1 + M2 + M3 + M5**: a loaded index, the read-only features, the linter, and a way to install it. The foundation specs move Draft → In Review when M0 implementation starts.

## Milestones

### M0 — Skeleton

A binary that initializes, scans a workspace, loads catalogs, and answers a hover with "I found this msgid". Proves the `tower-lsp-server` + tree-sitter + `polib` plumbing and the e2e harness end to end. The QA workflow ([F16](features/F16-release-ci.md)) lands here too, so every later commit is gated.

- Crate scaffold per [E02-folder-structure](foundations/E02-folder-structure.md) and [E03-tech-stack](foundations/E03-tech-stack.md)
- `initialize`/`initialized`, `didOpen`/`didChange`/`didSave`/`didClose`, workspace scan, transports
- pytest-lsp e2e harness with an `LspClient` fixture ([E17-testing](foundations/E17-testing.md))

### M1 — Catalog index & extraction ([F01](features/F01-catalog-index.md), [F02](features/F02-message-extraction.md))

The foundation milestone: discover and load every `.po`/`.pot`, build the catalog index, and extract translation calls from Python and Jinja with tree-sitter. After M1 the server holds the whole project's messages in memory. Nothing is user-visible yet — this is the backbone the next milestone surfaces.

### M2 — Read-only features ([F04](features/F04-completion.md)–[F06](features/F06-navigation.md), [F08](features/F08-inlay-hints.md), [F09](features/F09-symbols.md))

The "open the shopfront and explore it" milestone: msgid completion, the hover translation table, goto/references/document-links, inlay-hint previews, and document + workspace symbols. Each is a pure lookup into the M1 index.

### M3 — Diagnostics ([F03](features/F03-diagnostics.md))

The linter milestone: the full diagnostic catalog — unknown msgids and f-strings on the source side, placeholder/plural/fuzzy/duplicate/obsolete and the new string-quality checks on the catalog side. Ships after the index because every check reads it. Low-cost string-level checks land first; the heavier ones (XML-tag, cross-locale) follow.

### M4 — Code actions & rename ([F07](features/F07-code-actions.md), [F10](features/F10-rename.md))

Editing lands: the catalog quick fixes (copy msgid, toggle fuzzy, fix placeholder, add plural forms) and msgid rename across catalogs and call sites. The diagnostic-attached quick fixes ride on M3's catalog.

### M5 — Editor packaging & release ([F14](features/F14-editor-integration.md), [F16](features/F16-release-ci.md))

The Zed extension, Neovim and Helix config, and the release-on-tag workflow that cross-compiles binaries and attaches them to a GitHub Release. **First public release = M1 + M2 + M3 + M5.**

### M6 — Hardcoded-string detection & extract ([F11](features/F11-hardcoded-strings.md))

The headline feature: flag user-facing literals not wrapped in `_()`, and a code action to wrap them and add the msgid to the template. Implements the config flag the legacy server left as a stub.

### M7 — Code lens ([F12](features/F12-code-lens.md))

Usage counts on call sites and coverage lenses ("2 of 3 locales") on catalog entries, reading the same index.

### M8 — Catalog commands ([F13](features/F13-catalog-commands.md))

`extract` / `update` / `compile` wired as `workspace/executeCommand`, surfaced through code actions at natural locations, with the CLI as the editor-agnostic path.

### M9 — CLI check mode ([F15](features/F15-cli.md))

The `lsp`/`check`/`extract`/`update`/`compile`/`stats` subcommand split, the ruff-style `--output-format` set, `check --fix` applying the deterministic F07 fixes headless, the `stats` coverage report, and the shared-engine parity tests proving `check` and the server publish identical diagnostics.

## Sequencing rules

- M2 and M3 each depend only on M1 and can be reordered if priorities shift.
- M4's quick fixes attach to M3's diagnostic catalog; rename needs only M1.
- M5 can start any time after M1 produces a useful binary; the release half needs the binary, the QA half lands at M0.
- M6 reuses M2's extraction and M4's code-action plumbing.
- M8's commands reuse M4's code-action surface; the CLI half overlaps M9.
- M9 depends on M3 — it reuses the diagnostics engine wholesale.

## Cross-References

- **Related:** [01-overview](01-overview.md), [index](index.md).

## Changelog

- **2026-06-15** — Initial roadmap: M0–M9, first public release defined as M1 + M2 + M3 + M5, QA workflow carved into M0.
