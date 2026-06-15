# Constitution

> **Status:** Approved
>
> **Version:** 1.0   ·   **Last updated:** 2026-06-15
>
> **Purpose:** The governing rules for both the product and its specs — the principles babel-lsp must honor, and the conventions every spec in this suite follows.

---

## 1. Purpose & Scope

This document governs two things: the non-negotiable principles the language server is built on, and the authoring rules for every spec in this suite. When a spec and the constitution disagree, the constitution wins — fix the spec.

babel-lsp is a language server for **Python Babel internationalization**. It reads your gettext translation calls in Python and Jinja2, indexes your `.po`/`.pot` catalogs, and links the two — so your editor can navigate, complete, validate, and refactor translations the way a translation engineer would.

## 2. Product Principles

These are the rules the server must honor in every feature. Specs cite them as "per P3".

| # | Principle | What it means |
|---|---|---|
| P1 | Static analysis only | The server never imports, executes, or introspects user code. Source facts come from tree-sitter; catalog facts come from parsing `.po`/`.pot` text. Nothing runs. |
| P2 | Editor-agnostic | Every feature ships as a standard LSP capability over stdio. Zed, Neovim, and Helix are the first-class targets; no feature may depend on one editor's proprietary API. |
| P3 | Never panic on partial code | Users edit mid-keystroke and catalogs are often half-written. Extractors return partial facts for broken syntax and malformed entries; the server stays up and useful. |
| P4 | Only diagnose what is positively wrong | A diagnostic fires only when the facts prove something is incorrect — an unknown msgid, a placeholder mismatch. Unresolvable or incomplete input gets silence, not guesses. |
| P5 | The catalog is the source of truth | A msgid means whatever the catalogs say it means. The server links source calls to catalog entries; it complements the translator's `pybabel` workflow, it never replaces it. |
| P6 | Fast enough to forget it's there | The workspace scan and catalog load finish in seconds; recomputation after an edit is debounced and reads an in-memory index, never a re-parse of the world. |

## 3. Engineering Principles

- **Two-pass indexing.** Pass 1 extracts facts — translation calls from source files, entries from catalog files. Pass 2 links them: every `(msgid, msgctxt)` key resolves to its entries across locales and domains. Per-file work happens on every keystroke; the catalog index is rebuilt on catalog change, debounced.
- **Features are pure functions.** Every LSP capability is a function of the shared state plus a position. No feature holds mutable state of its own.
- **One parser per language, no regex.** tree-sitter parses Python *and* Jinja2; `polib` parses catalogs. The legacy server extracted Jinja calls with regex — this suite replaces that with a tree-sitter Jinja grammar for robustness (per the rejected alternative below).
- **Unsaved buffers overlay the disk.** An open, unsaved `.po` buffer shadows its on-disk copy in the index, so diagnostics and completions reflect what you see, not what was last saved.
- **Unresolved is a first-class state.** A translation call whose msgid can't be read statically (a non-constant first argument) is kept and marked, then excluded from msgid checks rather than guessed at (per P4).
- **Boring, proven shape.** One binary, `tower-lsp-server` + tree-sitter + `polib` + `DashMap` state — the shape production Rust language servers converge on. Diverge only with a recorded decision.

**Rejected: regex-based Jinja extraction.** The legacy babel-lsp extracted `{{ _(...) }}` and `{% trans %}` blocks with regular expressions. It worked for the common case but broke on nested braces, escaped quotes, and multi-line blocks. We considered porting it as-is and rejected it: a tree-sitter Jinja grammar costs more up front but gives the same error-tolerant, position-accurate parsing the Python path already enjoys, and keeps the "no regex extraction" rule whole. Recorded so the trade isn't re-litigated.

## 4. Authoring Conventions

### 4.1 Document template

Every spec follows the suite template: the metadata header, then the numbered sections. Required sections are Purpose, Detailed Specification, Cross-References, and Changelog.

### 4.2 Naming & ID schemes

- **Files:** prefix + number + kebab slug. `E##` engineering foundations, `F##` features. The overview is `01-overview.md`; meta-docs are `index.md`, `constitution.md`, `glossary.md`. This suite has no UI, so the `D##` band is absent.
- **Reserved names:** foundation names follow the shared reserved-names registry — `E01` is always Architecture, `E07` always Data Model. Note that i18n is babel-lsp's *domain*, not a foundation of the tool, so there is no `E09-localization` spec; the domain lives in the glossary and in [F01](features/F01-catalog-index.md)/[F02](features/F02-message-extraction.md).
- **Requirement IDs:** each detailed spec declares a short uppercase tag (e.g. `CAT`); load-bearing rules are `REQ-CAT-01`, open questions `OQ-CAT-01`.
- **Diagnostic codes:** every diagnostic has a stable code in the form `area/short-name` (e.g. `po/duplicate-id`), defined in [F03-diagnostics](features/F03-diagnostics.md).

### 4.3 Crosslinking & the index

Specs link to each other inline and list every connection in their Cross-References section. The index is updated in the same edit as any spec change.

### 4.4 Status lifecycle & changelog

A spec moves `Draft → In Review → Approved`, and can end in one of two terminal states:

- **Deprecated** — was Approved, now superseded. Set the status and move the file to `deprecated/`.
- **Rejected** — considered and turned down. Set the status and move the file to `rejected/`.

Continuously-maintained meta docs — the index, glossary, and roadmap — carry a fourth status instead: **Living**. A Living doc never graduates through the lifecycle; it's kept current, updated in the same edit as the specs it tracks.

Archived specs keep their name; the index lists them so the trail stays visible. Every change gets a dated changelog entry.

## 5. The Recurring Example Cast

Every spec draws its examples from the same small project: **the shopfront app**, a workspace the specs return to again and again.

- **`app/views.py`** — Python views with translation calls: `_("Checkout")`, `pgettext("button", "Save")`, and `ngettext("%(num)d item", "%(num)d items", n)` for the cart count.
- **`app/templates/checkout.html`** — a Jinja2 template with `{{ _("Your cart") }}` and a `{% trans count=n %}One item{% pluralize %}{{ count }} items{% endtrans %}` block.
- **`locale/messages.pot`** — the extracted template catalog; every msgid the source uses appears here.
- **`locale/de/LC_MESSAGES/messages.po`** — the German catalog. `"Checkout"` is translated; `"Save"` is marked `#, fuzzy`.
- **`locale/fr/LC_MESSAGES/messages.po`** — the French catalog. `"Checkout"` has no translation yet.
- **`pyproject.toml`** — carries `[tool.babel-lsp]` config and names the locale directory.

When a spec needs a mistake to illustrate a diagnostic, it breaks the shopfront: a typo'd `_("Chekout")` that no catalog knows, a German `msgstr` reading `%(naam)d` where the msgid said `%(num)d`, a second `msgid "Checkout"`, or an f-string slipped into `_(f"Hello {user}")`.

## 6. Visualization Style Guide

- **Mermaid** for flows, lifecycles, and graphs — labeled arrows and a semantic color palette (see the `mermaid` skill). **Do not use `%%{init}%%` directives**; style nodes with `classDef`/`class` and plain theme defaults only.
- **Tables** for index catalogs, the diagnostic catalog, and decision matrices.
- **ASCII mockups** for *editor surfaces* — the hover card, the completion menu, an inlay hint — where showing the rendered shape is clearer than describing it. There are no full-application *screen* mockups; the product has no screens, only these small LSP surfaces.

## 7. Cross-References

- **Related:** [index](index.md), [glossary](glossary.md).

## 8. Changelog

- **2026-06-15** — Initial constitution: six product principles, the shopfront example cast, the rejected regex-Jinja alternative, and naming/diagnostic-code conventions.
