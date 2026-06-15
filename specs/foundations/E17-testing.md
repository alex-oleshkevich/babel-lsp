# E17 — Testing

> **Status:** Draft
>
> **Version:** 0.2   ·   **Last updated:** 2026-06-15
>
> **Purpose:** How the server is tested — Rust unit tests for the pure logic, and a real-client e2e harness driving the binary against fixture workspaces.
>
> **Depends on:** [E01-architecture](E01-architecture.md), [E02-folder-structure](E02-folder-structure.md)   ·   **Related:** [F03-diagnostics](../features/F03-diagnostics.md), [F15-cli](../features/F15-cli.md)

> Requirement tag: **TST**

---

## 1. Purpose & Scope

This spec defines the two test layers and the fixtures they share. It's the contract every feature spec's "how it's tested" note points back to.

## 2. Detailed Specification

### 2.1 Two layers

**REQ-TST-01 — Unit tests for pure logic, e2e tests for protocol behavior.**

Pure functions — msgid extraction, the path-to-`(locale, domain)` rule, placeholder comparison, plural counting, PO range math — are tested as Rust unit tests beside their code. Anything that crosses the LSP boundary — capability negotiation, diagnostics publishing, hover content, code-action edits — is tested end to end through a real client against the built binary.

**REQ-TST-02 — The e2e harness is `pytest-lsp`.**

The end-to-end suite drives the server with `pytest-lsp`, which speaks real JSON-RPC over stdio. Each test opens a fixture workspace, sends requests, and asserts on responses — the same path an editor takes. This catches the integration bugs unit tests can't: wrong capability flags, off-by-encoding ranges, notifications applied out of order.

### 2.2 Fixtures

**REQ-TST-03 — Fixtures are the shopfront, broken on purpose.**

The fixture corpus is the constitution's shopfront app and variants of it: a clean copy, a copy with a typo'd msgid, one with a placeholder mismatch in the German catalog, one with a duplicate `msgid`, one with an f-string in `_()`. Each diagnostic in [F03](../features/F03-diagnostics.md) has a fixture that triggers it at a known position, so a test can assert the exact range and code.

A dedicated **non-ASCII fixture** — catalogs full of multi-byte translations — pins the position-encoding edge cases from [E01 REQ-ARCH-09](E01-architecture.md): a hover range must land on the right character whether the client negotiated UTF-8 or UTF-16.

### 2.3 Protocol conformance

**REQ-TST-04 — Lifecycle, ordering, and cancellation are tested.**

The harness asserts the protocol conduct from [E01 §5.6](E01-architecture.md): a newly opened file always receives a publish (the "pass 2 ran" signal), a relink that clears a finding sends an empty publish, two rapid `didChange` events apply in order, and an in-flight request honors `$/cancelRequest`.

**REQ-TST-05 — CLI and server publish identical findings.**

A parity test runs `babel-lsp check` over a fixture and compares its findings — code, file, range — against what the server publishes for the same workspace ([F15](../features/F15-cli.md)). The two share one diagnostics engine; this test keeps them from drifting.

### 2.4 Performance

**REQ-TST-06 — The budgets are tested against a large fixture.**

A generated large workspace (thousands of calls, several locales) asserts the [E01 §8](E01-architecture.md) budgets: initial scan + load, relink time, and hover/completion latency. A regression that blows the budget fails CI.

### 2.5 Feature coverage

**REQ-TST-07 — Every feature capability has at least one e2e test.**

REQ-TST-01 says protocol behavior is tested end to end; this requirement makes that *complete and checkable*. Every feature spec owns at least one `pytest-lsp` test that exercises its surface against the shopfront, listed in the matrix below. A feature spec is not "done" until its row passes. Each feature spec carries a short **Testing** note pointing back here, so the contract is visible from both ends.

The matrix is the coverage contract — one canonical assertion per feature, all against the shopfront fixture:

| Feature | Canonical e2e assertion |
|---|---|
| [F01](../features/F01-catalog-index.md) catalog index | Opening the workspace indexes all three catalogs; a probe request resolves `Checkout` as known, `de` translated, `fr` missing. |
| [F02](../features/F02-message-extraction.md) extraction | The Python `_("Checkout")` and the Jinja `{% trans %}` block both surface as calls (asserted via references). |
| [F03](../features/F03-diagnostics.md) diagnostics | Every code has a triggering fixture (REQ-TST-03) — 100% of the catalog. |
| [F04](../features/F04-completion.md) completion | Typing `_("Che` returns a `Checkout` item with `[de] Kasse` detail and a correct `textEdit`. |
| [F05](../features/F05-hover.md) hover | Hover on `_("Checkout")` returns markdown with `de` (ok) and `fr` (missing) rows. |
| [F06](../features/F06-navigation.md) navigation | Goto returns locations in the `.pot` + both `.po`; references finds call + entries; a `#:` link resolves to the source line. |
| [F07](../features/F07-code-actions.md) code actions | "Remove fuzzy flag" on `Save` and "Copy msgid to msgstr" on empty `fr` `Checkout` each return the expected `WorkspaceEdit`. |
| [F08](../features/F08-inlay-hints.md) inlay hints | With `inlay_hint_locale = "de"`, `_("Checkout")` yields a hint ` = Kasse`; absent the config, none. |
| [F09](../features/F09-symbols.md) symbols | `documentSymbol` of `de.po` lists `Checkout`/`Save`; `workspace/symbol "chec"` finds `Checkout`. |
| [F10](../features/F10-rename.md) rename | `prepareRename` returns the msgid range; `rename` to `Checkout page` edits `views.py` + `.pot` + both `.po`. |
| [F11](../features/F11-hardcoded-strings.md) hardcoded strings | With `detect_hardcoded_strings = true`, `return "Order placed"` raises `msg/hardcoded-string`; the extract action wraps it and appends the `.pot`. |
| [F12](../features/F12-code-lens.md) code lens | The lens on `msgid "Checkout"` resolves to "1 of 2 locales translated"; on the call, "used 1 time". |
| [F13](../features/F13-catalog-commands.md) catalog commands | A `.pot` code action offers "Update from template"; `executeCommand babel-lsp.update` invokes a **faked `pybabel`** (REQ-TST-08) and republishes. |
| [F14](../features/F14-editor-integration.md) editor integration | A stdio smoke test: launching `babel-lsp lsp --stdio` (the Zed extension's command) answers `initialize` with the advertised capabilities. |
| [F15](../features/F15-cli.md) CLI | `check` on a broken fixture exits `1` with the documented `concise` line; `--output-format json` matches the shape; a clean fixture exits `0`. |
| [F16](../features/F16-release-ci.md) release & CI | Exercised by the workflows themselves running in GitHub Actions, not `pytest-lsp` — the QA job *is* the test. |

**REQ-TST-08 — External tools are faked in e2e tests.**

The catalog commands ([F13](../features/F13-catalog-commands.md)) shell out to `pybabel`/`msgfmt`. Tests inject a fake binary on `PATH` that records its arguments and writes a fixed catalog, so the e2e suite stays hermetic and fast — it asserts babel-lsp invokes the tool with the right arguments and reloads afterward, never that `pybabel` itself works.

## 3. Cross-References

- **Depends on:** [E01-architecture](E01-architecture.md) — the behaviors under test; [E02-folder-structure](E02-folder-structure.md) — the `tests/e2e/` tree.
- **Related:** [F03-diagnostics](../features/F03-diagnostics.md) — the per-code fixtures; [F15-cli](../features/F15-cli.md) — the parity test; the per-feature matrix (§2.5) links every other feature spec.

## 4. Changelog

- **2026-06-15** — v0.2: added REQ-TST-07, the per-feature e2e **coverage matrix** (one canonical assertion per F01–F16) that makes "100% feature coverage" a checkable contract, and REQ-TST-08, the faked-`pybabel` rule keeping the suite hermetic. Each feature spec now carries a Testing note pointing at its matrix row.
- **2026-06-15** — Initial draft: the unit/e2e split, the `pytest-lsp` harness, the shopfront fixture corpus with a non-ASCII variant, protocol-conformance and CLI-parity tests, and the performance budget test.
