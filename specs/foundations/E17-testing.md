# E17 — Testing

> **Status:** Draft
>
> **Version:** 0.3   ·   **Last updated:** 2026-06-15
>
> **Purpose:** How babel-lsp is tested — the coverage policy, the test categories, the tools, and the shared fixtures every feature reuses. Each feature's own plan lives in its spec's §11 and links here.
>
> **Depends on:** [constitution](../constitution.md), [E02-folder-structure](E02-folder-structure.md)   ·   **Related:** [E29-e2e-testing](E29-e2e-testing.md), [E03-tech-stack](E03-tech-stack.md)

> Requirement tag: **TST**

---

## 1. Purpose & Scope

This spec defines how the whole server is tested and what "tested" means here. It is the authority every feature's **Testing** section (§11) defers to.

This spec covers:

- The coverage policy every feature must meet.
- The categories of test — unit and integration — and when to use each.
- The tools the suite standardizes on, including how external tools are faked.
- The shared **fixtures registry** — the shopfront workspaces, defined once and linked everywhere.
- Requirement-traceability conventions.

Out of scope: end-to-end protocol journeys, which have their own foundation — see [E29-e2e-testing](E29-e2e-testing.md).

## 2. Coverage Policy

The non-negotiable bar every feature is written against.

**REQ-TST-01 — Every feature is 100% covered.**

Each feature ships a test plan (its spec's §11) covering **all of its behavior**: every `REQ-<TAG>-NN` maps to at least one test, and every editor-surface state (§6) and edge case (§10) has a test. A feature with uncovered behavior is not done.

**REQ-TST-02 — Coverage is traceable, not just numeric.**

Coverage is demonstrated by the requirement-coverage table in each feature's §11.4, not only a line-coverage percentage. A green percentage with an untested requirement still fails the bar. Each feature's §11.4 is the index from a `REQ-<TAG>-NN` to the test that proves it.

## 3. Test Categories

Two categories live here; protocol journeys live in [E29](E29-e2e-testing.md).

| Category | Use it for | Speed / scope |
|---|---|---|
| **Unit** | Pure logic with no I/O — msgid extraction, the path-to-`(locale, domain)` rule, placeholder comparison, plural counting, PO range/escape math. Rust `#[cfg(test)]` modules beside the code. | Fast, isolated. |
| **Integration** | Several layers wired together without the LSP boundary — discovery → `polib` load → index build → a feature's pure-function read. Asserts the two-pass pipeline produces the right `CatalogIndex`. | Slower, in-process. |

End-to-end tests — a real client driving the built binary over stdio — are **not** here; they are [E29](E29-e2e-testing.md)'s, and they carry capability negotiation, publishing, and the editor-facing surfaces.

## 4. Tools & Frameworks

The standard toolchain; versions are pinned in [E03-tech-stack](E03-tech-stack.md).

- **Test runner:** `cargo test` for unit and integration; `pytest` + `pytest-lsp` for the E29 layer.
- **Assertions:** standard Rust assertions; snapshot tests (`insta`) for rendered hover/CLI output where exactness matters.
- **Fakes over mocks:** prefer real fixtures (real `.po` files, real parse trees) to mocks. The one faked dependency is the external `pybabel`/`msgfmt` binary (REQ-TST-04), so the suite stays hermetic.
- **Coverage reporter:** `cargo llvm-cov` in CI ([F16](../features/F16-release-ci.md)); the gate is the per-feature §11.4 tables, not a bare percentage.

**REQ-TST-04 — External tools are faked.**

The catalog commands ([F13](../features/F13-catalog-commands.md)) shell out to `pybabel`/`msgfmt`. Tests inject the [fake-pybabel](#fake-pybabel) fixture on `PATH` so the suite asserts babel-lsp invokes the tool with the right arguments and reloads afterward — never that `pybabel` itself works.

## 5. Fixtures Registry

The canonical home for reusable test data. Each fixture has a stable heading so a feature deep-links it from its §11.3 — e.g. `[the broken-placeholder catalog](../foundations/E17-testing.md#placeholder-mismatch)`.

### clean-shopfront

The constitution's shopfront workspace, fully wired and consistent: `app/views.py`, `app/templates/checkout.html`, `locale/messages.pot`, and the `de`/`fr` catalogs. `de` translates `Checkout`; `fr` leaves it missing; `de`'s `Save` is `#, fuzzy`. The baseline every feature reads from.

### unknown-msgid

clean-shopfront with a typo'd `_("Chekout")` in `views.py` that no catalog knows — triggers `msg/unknown-id`. Reused by [F03](../features/F03-diagnostics.md), [F05](../features/F05-hover.md).

### placeholder-mismatch

clean-shopfront whose German `msgstr` reads `%(naam)d` where the msgid said `%(num)d` — triggers `po/format-mismatch`. Reused by [F03](../features/F03-diagnostics.md), [F07](../features/F07-code-actions.md).

### duplicate-id

A catalog with `msgid "Checkout"` twice — triggers `po/duplicate-id`. Reused by [F03](../features/F03-diagnostics.md).

### fstring-call

clean-shopfront with `_(f"Hello {user}")` — triggers `msg/fstring-in-call` and exercises the unresolved-msgid path. Reused by [F02](../features/F02-message-extraction.md), [F03](../features/F03-diagnostics.md).

### non-ascii-catalog

Catalogs full of multi-byte translations, pinning the position-encoding edge cases ([E01 REQ-ARCH-09](E01-architecture.md)): a hover or rename range must land on the right character whether the client negotiated UTF-8 or UTF-16. Reused by [F05](../features/F05-hover.md), [F06](../features/F06-navigation.md), [F10](../features/F10-rename.md).

### large-workspace

A generated workspace — thousands of calls across several locales — for the [E01 §8](E01-architecture.md) performance budgets: initial scan + load, relink time, hover/completion latency. A regression that blows a budget fails CI.

### fake-pybabel

A stub `pybabel`/`msgfmt` placed on `PATH` that records its arguments and writes a fixed catalog, so command tests ([F13](../features/F13-catalog-commands.md), [F15](../features/F15-cli.md)) run without the real Babel toolchain (REQ-TST-04). Reused by [F13](../features/F13-catalog-commands.md), [F15](../features/F15-cli.md).

## 6. Conventions

**REQ-TST-03 — Requirement traceability.**

Every load-bearing `REQ-<TAG>-NN` is named in the test that verifies it, so a reader traces a rule to its proof and back. Each feature's §11.4 table is the index of this mapping.

- **Naming:** a test is named for the requirement and behavior it covers — `req_cat_01_discovers_po_under_locale_dir`.
- **Structure:** arrange / act / assert; one behavior per test.
- **Fakes vs mocks:** real fixtures by default; the only fake is the external `pybabel` (REQ-TST-04).
- **Where feature tests link:** every feature's §11 links here for categories, tools, and fixtures rather than restating them.

**REQ-TST-05 — `check` and the server publish identical findings.**

A parity test runs `babel-lsp check` over a fixture and compares its findings — code, file, range — against what the server publishes for the same workspace ([F15](../features/F15-cli.md)). The two share one diagnostics engine; this keeps them from drifting, and it extends to `check --fix` versus the editor's quick fixes.

## 7. Running Tests & CI

`cargo test` runs unit + integration locally; `pytest tests/e2e` runs the E29 layer (it needs the built binary). The `qa.yml` workflow ([F16](../features/F16-release-ci.md)) runs both on every push and PR over the MSRV and stable toolchains, plus the coverage report. A failing test or an uncovered requirement (§2) blocks merge.

## 8. Cross-References

- **Depends on:** [constitution](../constitution.md) — the coverage principles this enforces; [E02-folder-structure](E02-folder-structure.md) — the `tests/` tree.
- **Related:** [E29-e2e-testing](E29-e2e-testing.md) — the protocol-journey foundation; [E03-tech-stack](E03-tech-stack.md) — pinned tool versions; [F03](../features/F03-diagnostics.md), [F15](../features/F15-cli.md) — the per-code fixtures and the parity test.

## 9. Changelog

- **2026-06-15** — v0.3: restructured to the spec-writer testing-foundation template — split into the §2 coverage policy and the §5 fixtures registry (named, deep-linkable shopfront fixtures) that features link from their §11; moved the per-feature coverage matrix into each feature's §11.4; carved end-to-end protocol journeys out to the new [E29](E29-e2e-testing.md). Kept the CLI/server parity (REQ-TST-05) and faked-`pybabel` (REQ-TST-04) rules.
- **2026-06-15** — v0.2: added the per-feature e2e coverage matrix and the faked-pybabel rule (now superseded by the §5 registry and E29).
- **2026-06-15** — Initial draft: the unit/e2e split, the pytest-lsp harness, the shopfront fixture corpus, and the protocol-conformance, CLI-parity, and performance tests.
