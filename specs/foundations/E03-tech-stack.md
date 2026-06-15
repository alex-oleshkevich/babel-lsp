# E03 — Tech Stack

> **Status:** Draft
>
> **Version:** 0.2   ·   **Last updated:** 2026-06-15
>
> **Purpose:** The dependencies, language version, and toolchain the server is built on — and why each was chosen.
>
> **Depends on:** [constitution](../constitution.md)   ·   **Related:** [E01-architecture](E01-architecture.md), [E02-folder-structure](E02-folder-structure.md)

> Requirement tag: **TECH**

---

## 1. Purpose & Scope

This spec pins the crates and toolchain. It's the place to check before adding a dependency, and the reference for the version assumptions the rest of the suite makes.

## 2. Detailed Specification

### 2.1 Language and toolchain

**REQ-TECH-01 — Rust 2024, MSRV 1.85.**

The crate is edition 2024 with a minimum supported Rust version of 1.85 — the floor `tower-lsp-server` 0.23 requires. CI builds on stable; `rustfmt` and `clippy -D warnings` gate every push ([F16](../features/F16-release-ci.md)).

### 2.2 Core dependencies

Each crate earns its place against constitution P1 (static analysis) and the "boring, proven shape" engineering principle.

| Crate | Version | Role | Why this one |
|---|---|---|---|
| `tower-lsp-server` | 0.23 | LSP framing, JSON-RPC, `LanguageServer` trait | The maintained community fork of `tower-lsp`; LSP 3.17, `ls-types` URIs, no `async_trait` macro. |
| `tokio` | 1 | Async runtime | What `tower-lsp-server` runs on; `spawn_blocking` for parse/index work. |
| `ropey` | 1 | Rope-backed document text | Cheap incremental edits and clean UTF-8/UTF-16 offset math ([E01 REQ-ARCH-09](E01-architecture.md)). |
| `dashmap` | 6 | Concurrent document map | Lock-free per-entry reads for the document store. |
| `polib` | 0.3 | `.po`/`.pot` parsing | A real gettext parser — no hand-rolled PO reader (per "one parser per language"). |
| `tree-sitter` | 0.25 | Source parsing | Error-tolerant parse trees for partial code (P3). |
| `tree-sitter-python` | 0.25 | Python grammar | Resolves translation calls precisely, aliases and attribute access included. |
| `tree-sitter-jinja2` | [`alex-oleshkevich/tree-sitter-jinja2`](https://github.com/alex-oleshkevich/tree-sitter-jinja2) | Jinja grammar | Replaces the legacy regex Jinja extractor (constitution's rejected alternative); maintained in-house, so the staleness risk of a third-party grammar doesn't apply. |
| `globset` | 0.4 | Locale/catalog discovery | Glob matching for catalog discovery and the watched-file set. |
| `notify` | 6 | Native file watching | The fallback when the client can't register `didChangeWatchedFiles` ([E01 REQ-ARCH-12](E01-architecture.md)). |
| `clap` | 4 | CLI parsing | The `lsp`/`check`/`extract`/`update`/`compile` subcommands ([F15](../features/F15-cli.md)). |
| `serde` / `serde_json` / `toml` | 1 / 1 / 0.8 | Config + JSON-RPC payloads | Config deserialization and LSP message shapes. |
| `tracing` (+ `-subscriber`, `-appender`) | 0.1 | Structured logging | Logs to stderr or a file, never to stdout (which carries JSON-RPC). |

### 2.3 The `Uri` type

**REQ-TECH-02 — URIs go through `UriExt`, never string-munged.**

`tower-lsp-server` 0.23 uses the `ls-types` `Uri` (a `fluent_uri` newtype), not the old opaque `lsp-types` one. Path conversion uses `UriExt::from_file_path` / `UriExt::to_file_path` through one utility module; nothing else builds a URI by string formatting. This keeps Windows paths and percent-encoding correct in one place.

### 2.4 External tooling

**REQ-TECH-03 — `pybabel` is invoked, never reimplemented.**

The catalog commands ([F13](../features/F13-catalog-commands.md)) shell out to the user's `pybabel` (and `msgfmt` for compile) rather than reimplementing extraction in Rust. This respects P5 — the Babel toolchain stays the source of truth — and keeps babel-lsp's own surface to reading and validating. The binary is discovered from the project's virtualenv or `PATH`; absence is a graceful degradation, not a crash.

## 3. Open Questions & Decisions

- **Decision (resolves OQ-TECH-1)** — The Jinja grammar is [`alex-oleshkevich/tree-sitter-jinja2`](https://github.com/alex-oleshkevich/tree-sitter-jinja2), maintained in-house. It parses `{{ … }}` expressions and `{% … %}` blocks — including `{% trans %}`/`{% pluralize %}` — which is exactly what [F02](../features/F02-message-extraction.md) extraction needs. Because we own it, gaps are fixed upstream rather than worked around. The extraction contract stays grammar-agnostic, so the choice remains swappable if needs change.
- **Decision** — `polib` over a hand-written PO parser: gettext's escaping, multiline strings, and obsolete/fuzzy markers are subtle; a proven parser is worth the dependency.
- **Decision** — `ropey` added over the legacy server's plain `String` documents, to make incremental sync and encoding math clean from the start.

## 4. Cross-References

- **Depends on:** [constitution](../constitution.md) — P1 and the "boring shape" principle.
- **Related:** [E01-architecture](E01-architecture.md) — how these crates compose; [E02-folder-structure](E02-folder-structure.md) — where they're used; [F13](../features/F13-catalog-commands.md) — the `pybabel` integration; [F16](../features/F16-release-ci.md) — the toolchain gates.

## 5. Changelog

- **2026-06-15** — v0.2: resolved OQ-TECH-1 — the Jinja grammar is the in-house [`alex-oleshkevich/tree-sitter-jinja2`](https://github.com/alex-oleshkevich/tree-sitter-jinja2); updated the dependency row accordingly.
- **2026-06-15** — Initial draft: Rust 2024/MSRV 1.85, the `tower-lsp-server` 0.23 + tree-sitter + `polib` stack, `ropey` for documents, `UriExt` rule, and the `pybabel`-invocation decision.
