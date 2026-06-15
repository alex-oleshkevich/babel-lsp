# F13 — Catalog Commands

> **Status:** Draft
>
> **Version:** 0.3   ·   **Last updated:** 2026-06-15
>
> **Purpose:** Run pybabel-style extract, update, and compile from the editor — wired as LSP commands and surfaced where you'd reach for them.
>
> **Depends on:** [E03-tech-stack](../foundations/E03-tech-stack.md), [F07-code-actions](F07-code-actions.md)   ·   **Related:** [F14-editor-integration](F14-editor-integration.md), [F15-cli](F15-cli.md)

> Requirement tag: **CMD**

---

## 1. Purpose & Scope

The quick fixes in [F07](F07-code-actions.md) edit one catalog entry. This spec is the other axis: the whole-catalog operations that keep the catalogs in sync with your source — extract, update, compile.

You trigger these from the editor, but the server never re-implements them. It shells out to your `pybabel`, watches the result, and reloads the catalogs. The catalog stays the source of truth (constitution P5); babel-lsp just drives the tool you already use.

This spec covers:

- The three commands — extract, update, compile — declared at `initialize`
- How an editor with no command palette (Zed) triggers them, through code actions
- Running `pybabel`, reporting progress, and relinking on success
- The CLI fallback when an editor can't trigger commands at all

## 2. Non-Goals / Out of Scope

- **Re-implementing extraction, merge, or compile in Rust.** Per constitution P5 and [E03 REQ-TECH-03](../foundations/E03-tech-stack.md), `pybabel` (and `msgfmt`) do the work. babel-lsp reads and validates; it never becomes a second Babel.
- Per-entry quick fixes — copy msgid, toggle fuzzy, add plural forms — owned by [F07](F07-code-actions.md). Those are local `WorkspaceEdit`s, not catalog-wide runs.
- Config resolution — where `pybabel_path`, the locale dir, and `babel.cfg` come from — owned by [E15-app-config](../foundations/E15-app-config.md). This spec consumes that config.
- The headless CLI surface itself — owned by [F15-cli](F15-cli.md). This spec describes the same three operations as editor commands.

## 3. Background & Rationale

A translation workflow has three whole-catalog steps. You **extract** msgids from source into a `.pot` template. You **update** each locale's `.po` by merging the new template in. You **compile** the `.po` files to binary `.mo` for the runtime. These are exactly what `pybabel extract`, `pybabel update`, and `pybabel compile` do.

The server already knows your catalogs and your config, so it can offer these as one-click commands — no remembering flags. The hard part is not running `pybabel`; it's *triggering* the command in an editor that has no generic way to invoke an arbitrary LSP command. That triggering model is the heart of this spec.

## 4. Concepts & Definitions

- **Server command** — a string id the server registers at `initialize`; the client invokes it via `workspace/executeCommand`, and the server's `execute_command` handler runs it. Distinct from a code action's inline `WorkspaceEdit` ([F07](F07-code-actions.md)), which the client applies with no round-trip.
- **Trigger surface** — the UI an editor renders that can carry a command. For babel-lsp the surface is the code-action lightbulb (canonical in [F07](F07-code-actions.md)).
- **Catalog**, **POT template**, **domain**, **locale** — canonical in the [glossary](../glossary.md).

## 5. Detailed Specification

### 5.1 The three commands

Each command is one whole-catalog `pybabel` operation. Read them as verbs on the catalog set.

**REQ-CMD-01 — Three commands, declared at initialize.**

The server advertises exactly three command ids in `ServerCapabilities.execute_command_provider`:

```rust
// src/server/commands.rs — registered in the initialize result
ExecuteCommandOptions {
    commands: vec![
        "babel-lsp.extract".into(),  // pybabel extract → write/update the .pot
        "babel-lsp.update".into(),   // pybabel update  → merge .pot into each .po
        "babel-lsp.compile".into(),  // pybabel compile → .po → .mo
    ],
    ..Default::default()
}
```

A client that reads this list knows the server will honour those three `workspace/executeCommand` calls. Each maps to one subprocess invocation (§5.4).

| Command id | pybabel call | Effect |
|---|---|---|
| `babel-lsp.extract` | `pybabel extract -F babel.cfg -o messages.pot .` | Re-scan source, rewrite the `.pot` template |
| `babel-lsp.update` | `pybabel update -i messages.pot -d locale` | Merge the template into every locale's `.po` |
| `babel-lsp.compile` | `pybabel compile -d locale` | Compile each `.po` to its `.mo` |

The exact flags derive from resolved config ([E15](../foundations/E15-app-config.md)) — the locale dir, the domain, the `babel.cfg` path. Each command optionally takes a locale argument, so "update just `fr`" is a narrower `workspace/executeCommand` with `arguments: ["fr"]`.

### 5.2 The trigger model

This is the part the question "how do I trigger it in Zed?" turns on. The short answer: through code actions, because Zed renders no command palette for arbitrary LSP commands.

**REQ-CMD-02 — Commands are triggered through code actions, not a command palette.**

VS Code lets an extension contribute a command to its palette, so a user can type "Babel: Extract" and fire `workspace/executeCommand` directly. Zed, Neovim, and Helix have no such generic contribution point — there is no menu listing the server's registered command ids.

So the server reaches `workspace/executeCommand` through the one trigger surface every editor already renders: the **code-action lightbulb**. The flow:

1. The server returns a `CodeAction` whose `command` field references a registered command id (e.g. `babel-lsp.compile`). The action carries no `edit`.
2. The user picks it from the lightbulb menu. The client sends `workspace/executeCommand` with that id and its arguments.
3. The server's `execute_command` handler runs the `pybabel` op (§5.4) and, for source-buffer changes, calls `client.apply_edit(WorkspaceEdit)` — otherwise it shells out and republishes diagnostics on completion.

A command action and a quick-fix action look the same to the user; the difference is that a command action defers the work to the server, where a quick fix ([F07 REQ-ACT-01](F07-code-actions.md)) carries its edit inline.

**REQ-CMD-03 — Command actions anchor at natural source locations.**

A whole-catalog op has no cursor of its own, so the server attaches the command action where you'd reach for it:

- Cursor in a `.po` or `.pot` file → **"Compile catalog"** and **"Update from template"**.
- Cursor in `pyproject.toml` or `babel.cfg` → **"Extract messages"**.

These ride the same `code_action` handler as [F07](F07-code-actions.md), but produce `CodeActionKind::SOURCE` actions carrying a `Command`, not a `WorkspaceEdit`. The handler offers them whenever the file type matches and the resolved config names a locale dir — no diagnostic precondition, since these are operations, not fixes. The menu shape is in §6.1.

**REQ-CMD-04 — The CLI is the reliable cross-editor trigger.**

When an editor can't surface a command at all — or in CI, where there is no editor — the `babel-lsp` CLI ([F15](F15-cli.md)) runs the same three operations headless: `babel-lsp extract`, `babel-lsp update`, `babel-lsp compile`. This is the recommended path whenever editor triggering is awkward; it reuses the same config resolution and the same `pybabel` invocation, so the result is identical to the command. The editor commands are a convenience over a CLI that always works.

**REQ-CMD-05 — Code lens is a secondary surface where supported.**

For clients that render `textDocument/codeLens`, the server may attach a command lens at the top of a `.pot` ("Update all catalogs") or a `.po` ("Compile"). The lens carries the same `Command` as the code action. It is additive — the code-action path (§5.2) is the baseline that every first-class editor supports.

### 5.3 The executeCommand handler

**REQ-CMD-06 — One handler dispatches the three ids; an unknown id is rejected.**

`execute_command` matches `params.command` against the three registered ids and dispatches to the matching runner (§5.4). Any other id returns an error response — the server only honours what it advertised in REQ-CMD-01. Arguments (an optional locale) are validated before the subprocess spawns.

```rust
// src/server/commands.rs
pub async fn execute_command(
    &self,
    params: ExecuteCommandParams,
) -> Result<Option<Value>> {
    match params.command.as_str() {
        "babel-lsp.extract" => self.run_extract(args).await,
        "babel-lsp.update"  => self.run_update(args).await,
        "babel-lsp.compile" => self.run_compile(args).await,
        other => Err(unknown_command(other)),
    }
}
```

### 5.4 Running pybabel

**REQ-CMD-07 — pybabel is spawned; its path comes from config or PATH.**

The runner spawns `pybabel` (and `msgfmt` for compile, when configured to use it) as a child process under `tokio`. The binary path is the resolved `pybabel_path` from [E15](../foundations/E15-app-config.md), falling back to the first `pybabel` on `PATH`. The server passes the config-derived flags (§5.1) and the workspace root as the working directory. Nothing about the user's program is imported or executed — only the Babel tool runs (constitution P1 is about *user code*; invoking the user's chosen toolchain is the explicit P5 exception in [E03 REQ-TECH-03](../foundations/E03-tech-stack.md)).

**REQ-CMD-08 — Progress streams over workDoneProgress; failures surface a message.**

When the client advertises `window.workDoneProgress`, the runner creates a progress token and reports the operation's phases — "Extracting messages…", "Updating fr…", "Compiling…". A long extract is therefore visible and cancellable (§10). The progress toast is in §6.2. On a non-zero exit, the runner surfaces the subprocess `stderr` via `window/showMessage` at error severity; it does not invent diagnostics from the tool's output.

**REQ-CMD-09 — On success the server reloads catalogs and relinks.**

When `pybabel` exits zero and has touched catalog files, the server re-runs pass 1 on the changed `.po`/`.pot` files and triggers a debounced pass 2 ([E01 REQ-ARCH-04](../foundations/E01-architecture.md)), then republishes diagnostics ([E01 REQ-ARCH-10](../foundations/E01-architecture.md)). An `extract` that rewrote the `.pot` clears `msg/unknown-id` squiggles on newly templated msgids; a `compile` touches no `.po` text, so it relinks nothing and only reports done. The file watcher ([E01 REQ-ARCH-12](../foundations/E01-architecture.md)) would catch the disk change anyway; the explicit reload just makes the command feel synchronous.

## 6. UI Mockups

The two surfaces a user sees: the lightbulb menu that carries the command, and the progress toast while `pybabel` runs. Both are rendered by the editor — babel-lsp only supplies the `CodeAction` and the `workDoneProgress` token.

### 6.1 Code-action lightbulb menu — anchored in a catalog

This is what the user sees when the cursor sits in `messages.pot` (or any `.po`) and they open the lightbulb. The command actions ride the same menu as [F07](F07-code-actions.md)'s quick fixes; here only the catalog-wide ones show.

```
locale/messages.pot
   12 │ msgid "Wishlist"
   13 │ msgstr ""
 💡 ◂ lightbulb on the msgid line
   ╭─────────────────────────────────╮
   │  Update from template           │  ◂ babel-lsp.update
   │  Compile catalog                │  ◂ babel-lsp.compile
   ╰─────────────────────────────────╯
```

States: in a `.po`/`.pot` → both rows · in `babel.cfg`/`pyproject.toml` → a single **"Extract messages"** row · no locale dir resolved → no command rows.

### 6.2 Work-done progress toast — while pybabel runs

This appears after the user picks a command, for as long as the subprocess runs. The editor renders it from the `window/workDoneProgress` token; the phase text comes from the runner (REQ-CMD-08), and the cancel control maps to `window/workDoneProgress/cancel`.

```
┌──────────────────────────────────────────────┐
│ ⟳ Updating catalogs…  <phase: fr>   [ Cancel ]│
└──────────────────────────────────────────────┘
```

States: running (phase text updates per locale) · success (toast clears, diagnostics republish) · error (toast clears, an error `showMessage` replaces it) · cancelled (toast clears, child process killed).

## 7. Visualizations

The path from lightbulb to relink, end to end. The phases are coloured: the editor round-trip, the server's command handling, and the subprocess + reload.

```mermaid
sequenceDiagram
    actor User
    participant Editor
    participant Server
    participant Pybabel as pybabel

    rect rgb(204, 229, 255)
    Note over User,Editor: Trigger phase
    User->>Editor: open lightbulb on messages.pot
    Editor->>Server: textDocument/codeAction
    Server-->>Editor: CodeAction "Update from template" (Command)
    User->>Editor: select the action
    Editor->>Server: workspace/executeCommand babel-lsp.update
    end

    rect rgb(212, 237, 218)
    Note over Server,Pybabel: Run phase
    Server->>Editor: window/workDoneProgress (begin)
    Server->>Pybabel: spawn pybabel update -i messages.pot -d locale
    Pybabel-->>Server: exit 0 (de.po, fr.po merged)
    end

    rect rgb(255, 243, 205)
    Note over Server,Editor: Reload phase
    Server->>Server: pass 1 + pass 2 relink
    Server->>Editor: textDocument/publishDiagnostics
    Server->>Editor: window/workDoneProgress (end)
    end
```

## 8. Data Shapes

The `workspace/executeCommand` request the client sends when a user picks a command action. The `command` is one of the three registered ids; `arguments` is an optional one-element array naming a single locale (omit it to run across all).

```json
{
  "command": "babel-lsp.update",
  "arguments": ["fr"]
}
```

## 9. Examples & Use Cases

You add `_("Wishlist")` to `app/views.py`, and `msgid "Wishlist"` exists in no catalog yet — `msg/unknown-id` squiggles it. You open `locale/messages.pot` and hit the lightbulb (§6.1). On the `.pot` it offers **"Update from template"** and **"Compile catalog"**; **"Extract messages"** is anchored on `babel.cfg`.

You run extract first from `babel.cfg`: `pybabel extract` re-scans source and rewrites `messages.pot` with `Wishlist` in it. Back in the `.pot`, you pick **"Update from template"**. The server runs `pybabel update` across `de.po` and `fr.po`, merging the new msgid into both as empty entries. The progress toast (§6.2) shows "Updating de…", "Updating fr…". On success it relinks, and `_("Wishlist")` loses its squiggle — now the catalogs know it, they just lack a translation (`po/missing-translation`, which [F07](F07-code-actions.md)'s copy-msgid fix scaffolds). Finally **"Compile catalog"** writes `de.mo` and `fr.mo`.

In CI, the same three steps are `babel-lsp extract && babel-lsp update && babel-lsp compile` — no editor, identical result.

## 10. Edge Cases & Failure Modes

- **`pybabel` not installed** → the spawn fails with `ENOENT`; the server surfaces "pybabel not found — install Babel or set `pybabel_path`" via `window/showMessage`, and the catalogs are untouched. No crash (constitution P3).
- **No virtualenv / wrong interpreter** → `pybabel_path` is unset and `PATH` has no `pybabel`; treated as not-installed, same graceful message. The fix is config, surfaced in the message.
- **Long-running extract on a big tree** → progress reports keep it visible; the work runs under `spawn_blocking`-adjacent async so the runtime never blocks ([E01 REQ-ARCH-08](../foundations/E01-architecture.md)). If the client sends `window/workDoneProgress/cancel`, the runner kills the child process and reports cancellation.
- **`pybabel` exits non-zero** (a malformed `babel.cfg`, an unreadable source file) → its `stderr` is shown verbatim; no partial reload, since the catalogs may be half-written. The next watcher event reconciles whatever did land.
- **Command id the server didn't register** → `execute_command` rejects it (REQ-CMD-06); a client can't invoke an operation babel-lsp never advertised.
- **A catalog open and unsaved in the editor** during update → `pybabel` edits the file on disk; the editor's buffer is now stale. The server's reload reads the unsaved overlay ([E07 REQ-IDX-07](../foundations/E07-data-model.md)) for indexing, but the user must reconcile the buffer with disk themselves — the server does not force-reload an editor buffer.

## 11. Testing

These commands shell out, so the suite leans on the faked `pybabel` throughout — every test asserts babel-lsp's behaviour around the subprocess, never that `pybabel` itself works.

### 11.1 Scope & coverage

Target: **100% of this feature's behavior is covered.** Every `REQ-CMD-NN` below maps to at least one test; every screen state (§6) and edge case (§10) has a test. See the policy in [E17 §2](../foundations/E17-testing.md#2-coverage-policy).

### 11.2 Test plan

| Behavior / scenario | Type | Fixtures | Verifies |
|---|---|---|---|
| Three command ids advertised in `execute_command_provider` at initialize | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-CMD-01 |
| Code action carries a `Command` (no inline edit), triggered through the lightbulb | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-CMD-02 |
| Command actions anchor at natural locations — `.po`/`.pot` → update/compile, `babel.cfg`/`pyproject.toml` → extract; §6.1 states | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-CMD-03 |
| `execute_command` dispatches each id to its runner; unknown id rejected | integration | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-CMD-06 |
| pybabel spawned with the config-derived args in the workspace dir; path from `pybabel_path` then `PATH` | integration | [fake-pybabel](../foundations/E17-testing.md#fake-pybabel) | REQ-CMD-07 |
| workDoneProgress begin/phase/end while running; §6.2 running state | integration | [fake-pybabel](../foundations/E17-testing.md#fake-pybabel) | REQ-CMD-08 |
| On success, catalogs reload and diagnostics republish (extract clears `msg/unknown-id`) | integration | [fake-pybabel](../foundations/E17-testing.md#fake-pybabel) | REQ-CMD-09 |
| CLI runs the same three ops headless with identical config resolution | integration | [fake-pybabel](../foundations/E17-testing.md#fake-pybabel) | REQ-CMD-04 |
| Code lens carries the same `Command` where the client supports codeLens | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-CMD-05 |
| pybabel not installed → graceful `showMessage`, catalogs untouched, no crash | integration | [fake-pybabel](../foundations/E17-testing.md#fake-pybabel) | REQ-CMD-07, REQ-CMD-08 |
| Cancel mid-run → child killed, cancellation reported; §6.2 cancelled state | integration | [fake-pybabel](../foundations/E17-testing.md#fake-pybabel) | REQ-CMD-08 |
| Non-zero exit → stderr shown verbatim, no partial reload; §6.2 error state | integration | [fake-pybabel](../foundations/E17-testing.md#fake-pybabel) | REQ-CMD-08, REQ-CMD-09 |

### 11.3 Fixtures

Reusable fixtures live in the [E17 fixtures registry](../foundations/E17-testing.md#5-fixtures-registry) — linked above. This feature needs no feature-local fixtures.

- **[fake-pybabel](../foundations/E17-testing.md#fake-pybabel)** — the stub `pybabel`/`msgfmt` placed on `PATH` that records its arguments and writes a fixed catalog, so every command test runs without the real Babel toolchain ([REQ-TST-04](../foundations/E17-testing.md)). The exit code and `stderr` are driven per-scenario to exercise the not-installed, non-zero, and cancel paths.
- **[clean-shopfront](../foundations/E17-testing.md#clean-shopfront)** — the baseline workspace the capability, anchoring, and dispatch tests read from.

### 11.4 Requirement coverage

Every load-bearing requirement maps to a test — this table is the proof.

| Requirement | Covered by |
|---|---|
| REQ-CMD-01 | `req_cmd_01_advertises_three_commands_at_initialize` |
| REQ-CMD-02 | `req_cmd_02_code_action_carries_command_not_edit` |
| REQ-CMD-03 | `req_cmd_03_actions_anchor_at_natural_locations` |
| REQ-CMD-04 | `req_cmd_04_cli_runs_same_three_ops_headless` |
| REQ-CMD-05 | `req_cmd_05_code_lens_carries_same_command` |
| REQ-CMD-06 | `req_cmd_06_dispatches_ids_rejects_unknown` |
| REQ-CMD-07 | `req_cmd_07_spawns_pybabel_with_config_args` |
| REQ-CMD-08 | `req_cmd_08_progress_and_failure_message` |
| REQ-CMD-09 | `req_cmd_09_reloads_and_relinks_on_success` |

## 12. End-to-End Test Plan

The journeys a real editor drives over stdio, using the faked `pybabel` ([REQ-TST-04](../foundations/E17-testing.md)) so a run never depends on a real Babel install.

### 12.1 Coverage target

**100% of the feature's scope, end to end** — the happy path plus all reasonably possible error paths (pybabel missing, non-zero exit, cancel). See the policy in [E29 §2](../foundations/E29-e2e-testing.md#2-coverage-policy).

### 12.2 Scenarios

| # | Journey | Path | Expected outcome |
|---|---|---|---|
| E2E-01 | `textDocument/codeAction` in `messages.pot` | happy | Response offers **"Update from template"** carrying `babel-lsp.update` |
| E2E-02 | `workspace/executeCommand babel-lsp.update` | happy | fake-pybabel spawned with `update -i messages.pot -d locale`; diagnostics republished after relink |
| E2E-03 | execute command with `pybabel` not installed | error | A clear window/`showMessage` error; server stays up, no crash, catalogs untouched |
| E2E-04 | cancel mid-run via `window/workDoneProgress/cancel` | error | Child process killed; cancellation reported; no partial reload |

### 12.3 Acceptance criteria & Definition of Done

The §12.2 scenarios, written Given/When/Then, are this feature's acceptance criteria:

| # | Given | When | Then |
|---|---|---|---|
| AC-01 | clean-shopfront open, cursor in `messages.pot` | the client requests `textDocument/codeAction` | the response includes **"Update from template"** carrying `babel-lsp.update` |
| AC-02 | clean-shopfront with fake-pybabel on `PATH` | the client sends `workspace/executeCommand babel-lsp.update` | fake-pybabel runs with the resolved args and diagnostics republish on relink |
| AC-03 | a workspace with no `pybabel` on `PATH` and `pybabel_path` unset | the client executes a command | a clear error `showMessage` arrives, the server stays up, and the catalogs are untouched |
| AC-04 | a long update in flight under fake-pybabel | the client sends `window/workDoneProgress/cancel` | the child is killed, cancellation is reported, and no partial reload happens |

**Definition of Done:** every `REQ-CMD-NN` has a passing test (§11.4), every acceptance scenario above passes, and the §13.1 security concern is verified.

## 13. Non-Functional Requirements

### 13.1 Security & Privacy

This is the one babel-lsp feature that runs a subprocess, so the trust boundary is worth stating plainly.

- **Access & trust boundary** — the server spawns the user's *own* `pybabel`/`msgfmt`, discovered from the venv or `PATH` (`pybabel_path`, [E15](../foundations/E15-app-config.md)). babel-lsp executes a local tool the user already has installed and already runs by hand; nothing is downloaded or fetched. The boundary it crosses is "run a known local binary," not "introduce a new one."
- **Input & validation** — the subprocess arguments are built only from resolved config (locale dir, domain, `babel.cfg` path) and an optional locale name — never from untrusted catalog *content*. So there is no path for a malicious `.po`/`.pot` to inject a command or flag; catalog bytes are parsed, never shelled.
- **Data sensitivity** — no network and no PII. The subprocess runs in the workspace directory and touches only the project's own catalog files; nothing leaves the machine.
- **Baseline** — the only privileged action is spawning a configured local executable with a fixed argument shape, validated before spawn (REQ-CMD-06). The not-installed and non-zero paths fail closed with a message, never a crash (§10).

## 15. Open Questions & Decisions

- **Decision (resolves OQ-CMD-1)** — v1 ships the standard `workspace/executeCommand` path only, triggered through code actions. It covers Zed, Neovim, and Helix — every first-class editor. A custom `babel-lsp/runCommand` method would enable a command palette and structured results, but it's a non-standard method to version and maintain for a consumer that doesn't exist yet; revisit only if a richer client (e.g. a future VS Code extension, [F14](F14-editor-integration.md)) needs it.
- **Decision (resolves OQ-CMD-2)** — `compile` prefers `pybabel compile` for one consistent Babel toolchain across all three commands (and per P5), and falls back to `msgfmt` only when Babel's compile isn't available. So a project needs just `pybabel` in the common case, and the editor button behaves the same as `extract`/`update`.
- **Decision** — Editor commands and the CLI ([F15](F15-cli.md)) share one runner module, so the three operations behave identically whether triggered from a lightbulb or a shell. The editor path only adds progress reporting and the relink.

## 16. Cross-References

- **Depends on:** [E03-tech-stack](../foundations/E03-tech-stack.md) — REQ-TECH-03, `pybabel` invoked not reimplemented, and `pybabel_path` discovery; [F07-code-actions](F07-code-actions.md) — the code-action trigger surface these commands ride.
- **Related:** [F14-editor-integration](F14-editor-integration.md) — how Zed/Neovim/Helix surface the lightbulb and progress; [F15-cli](F15-cli.md) — the headless path running the same three operations; [E01-architecture](../foundations/E01-architecture.md) — pass 1/pass 2 reload, progress, and watcher reconciliation; [E15-app-config](../foundations/E15-app-config.md) — `pybabel_path`, locale dir, and `babel.cfg` resolution.
- **Testing:** [E17 §2](../foundations/E17-testing.md#2-coverage-policy) — the coverage policy §11 defers to, with the faked `pybabel` per [REQ-TST-04](../foundations/E17-testing.md); [E29 §2](../foundations/E29-e2e-testing.md#2-coverage-policy) — the E2E coverage policy §12 defers to.

## 17. Changelog

- **2026-06-15** — v0.3: restructured to the spec-writer template — added §6 UI Mockups (the lightbulb command menu and the workDoneProgress toast), §11 Testing (coverage link, plan, fixtures pointing at fake-pybabel, and a REQ-CMD-01..09 coverage table), §12 E2E (codeAction/executeCommand happy paths plus pybabel-missing and cancel error paths over the faked tool), and §13.1 Security (the subprocess trust boundary — a local tool the user already has, args from config never catalog content, no network/PII). Renumbered to canonical section order. Preserved all requirements and the resolved OQ-CMD-1/2 decisions.
- **2026-06-15** — v0.2: resolved the command open questions — standard `workspace/executeCommand` only, no custom `babel-lsp/runCommand` method (OQ-CMD-1); `compile` prefers `pybabel compile`, falling back to `msgfmt` (OQ-CMD-2). Added the E17 coverage-matrix testing note.
- **2026-06-15** — Initial draft: the three `babel-lsp.{extract,update,compile}` commands declared at initialize; the code-action trigger model for palette-less editors (Zed), with natural-location anchoring and the CLI as the reliable cross-editor fallback; the `execute_command` dispatch, `pybabel` subprocess runner, workDoneProgress, and success-relink; not-installed/cancel/non-zero edge cases; the `babel-lsp/runCommand` extension open question.
</content>
</invoke>
