# babel-lsp — Specification Index

> **Status:** Living (continuously maintained)
>
> **Last updated:** 2026-06-15
>
> **Purpose:** The map of the whole specification suite — every spec, what it defines, when to load it, and how finished it is. Start here.

babel-lsp is a Rust language server for Python Babel internationalization. The suite is organized in four tiers: meta docs that govern the rest, product docs that orient newcomers, engineering foundations that describe how the server is built, and feature specs that describe what each capability does.

**Foundation specs describe _how_ the server is built. Feature specs describe _what_ each feature does** and own their diagnostic codes, commands, and editor surfaces.

## Status legend

✅ Approved · 📝 In Review · ✏️ Draft · 🔄 Living · ♻️ Deprecated · ⛔ Rejected

## Tier 1 — Meta

| Spec | Purpose | Load this when | Status |
|---|---|---|---|
| [constitution](constitution.md) | Governing principles and authoring conventions | Writing or reviewing any spec | ✅ |
| [glossary](glossary.md) | Canonical definition of every domain term | A term is unclear | 🔄 |

## Tier 2 — Product

| Spec | Purpose | Load this when | Status |
|---|---|---|---|
| [01-overview](01-overview.md) | What the server is, in plain language | Onboarding to the project | ✏️ |
| [roadmap](roadmap.md) | Build order — milestones M0–M9 | Planning what to build next | 🔄 |

## Tier 3 — Foundations

| Spec | Purpose | Load this when | Status |
|---|---|---|---|
| [E01-architecture](foundations/E01-architecture.md) | Two-pass model, process model, LSP plumbing | Understanding how it all fits | ✏️ |
| [E02-folder-structure](foundations/E02-folder-structure.md) | Source/test layout and layering | Adding a module | ✏️ |
| [E03-tech-stack](foundations/E03-tech-stack.md) | Dependencies, toolchain, versions | Touching dependencies | ✏️ |
| [E07-data-model](foundations/E07-data-model.md) | Catalog index and document state | Touching shared state | ✏️ |
| [E15-app-config](foundations/E15-app-config.md) | Config sources, locale discovery, rule toggles | Reading or adding a setting | ✏️ |
| [E17-testing](foundations/E17-testing.md) | Coverage policy, categories, fixtures registry | Writing a feature test plan | ✏️ |
| [E29-e2e-testing](foundations/E29-e2e-testing.md) | E2E coverage policy, pytest-lsp harness, patterns | Writing a feature E2E plan | ✏️ |

## Tier 4 — Features

Domain specs own indexing semantics; capability specs own one LSP surface; delivery specs ship the binary.

| Spec | Purpose | Load this when | Status |
|---|---|---|---|
| [F00-template](features/F00-template.md) | Boilerplate for new feature specs | Starting a new feature | — |
| [F01-catalog-index](features/F01-catalog-index.md) | Discover, load, and index catalogs | Touching the index | ✏️ |
| [F02-message-extraction](features/F02-message-extraction.md) | Parse translation calls (Python + Jinja) | Touching extraction | ✏️ |
| [F03-diagnostics](features/F03-diagnostics.md) | The diagnostic catalog and codes | Adding or changing a check | ✏️ |
| [F04-completion](features/F04-completion.md) | msgid completion in calls | Working on completion | ✏️ |
| [F05-hover](features/F05-hover.md) | Translation table on hover | Working on hover | ✏️ |
| [F06-navigation](features/F06-navigation.md) | Definition, references, document links | Working on navigation | ✏️ |
| [F07-code-actions](features/F07-code-actions.md) | Catalog quick fixes | Working on code actions | ✏️ |
| [F08-inlay-hints](features/F08-inlay-hints.md) | Inline translation previews | Working on inlay hints | ✏️ |
| [F09-symbols](features/F09-symbols.md) | Document + workspace symbols | Working on symbols | ✏️ |
| [F10-rename](features/F10-rename.md) | Rename a msgid everywhere | Working on rename | ✏️ |
| [F11-hardcoded-strings](features/F11-hardcoded-strings.md) | Detect + extract untranslated literals | Working on extraction quick fix | ✏️ |
| [F12-code-lens](features/F12-code-lens.md) | Usage and coverage lenses | Working on code lens | ✏️ |
| [F13-catalog-commands](features/F13-catalog-commands.md) | extract / update / compile commands | Working on commands | ✏️ |
| [F14-editor-integration](features/F14-editor-integration.md) | Zed, Neovim, Helix integration | Packaging for an editor | ✏️ |
| [F15-cli](features/F15-cli.md) | `check` and catalog subcommands | Working on the CLI | ✏️ |
| [F16-release-ci](features/F16-release-ci.md) | GitHub QA + release workflows | Touching CI | ✏️ |

## Deprecated

| Spec | Superseded by | Status |
|---|---|---|
| _none yet_ | | |

## Rejected

| Spec | Why rejected | Status |
|---|---|---|
| _none yet_ | | |

## Out of scope

The suite does not cover: machine translation, a translation-memory backend, a `.mo` runtime loader, or a GUI catalog editor. babel-lsp reads and validates; it never translates.

## Maintenance rule

When you author or change a spec, update its row here in the same edit. When a spec is **deprecated**, move it to `deprecated/` and list it above; when a proposal is **rejected**, move it to `rejected/` and list it.

## Changelog

- **2026-06-15** — Adopted the updated spec-writer structure: new [E29-e2e-testing](foundations/E29-e2e-testing.md) foundation; restructured [E17-testing](foundations/E17-testing.md) into a coverage policy + fixtures registry; constitution §4.4 Testing and §4.6 non-functional scope (Security required; Accessibility N/A — editor renders, content rule in §6; Acceptance Enabled; the rest N/A). Every feature spec (F01–F16) gained §11 Testing and §13.1 Security & Privacy; surface-bearing features gained §6 UI Mockups; user-facing features gained §12 E2E + §12.3 acceptance. No §13.2 — accessibility is the editor's.
- **2026-06-15** — Open-question resolution pass: decided 13 OQs across E01/E03/E15/F02/F03/F07/F13/F14/F15/F16 — stdio-only transport, `tree-sitter-jinja2`, `check --fix` + `stats`, structured Jinja placeholders, and the PyPI/AUR/Homebrew release (macOS unsigned, no crates.io). Three F11 hardcoded-strings OQs remain deferred. Also added the E17 e2e coverage matrix (REQ-TST-07/08), F01 external-change detection (REQ-CAT-09/10), the F03 lint provenance column, and F05 hover mockups. Bumped the touched specs.
- **2026-06-15** — Initial index: two meta docs, two product docs, six foundations, and sixteen feature specs (F01–F16) plus the template.
