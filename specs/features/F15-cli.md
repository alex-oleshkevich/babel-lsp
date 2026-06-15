# F15 — CLI

> **Status:** Draft
>
> **Version:** 0.2   ·   **Last updated:** 2026-06-15
>
> **Purpose:** The command-line surface — serve the LSP, and run the same diagnostics headless as a CI-friendly linter with ruff-style output.
>
> **Depends on:** [F03-diagnostics](F03-diagnostics.md), [E15-app-config](../foundations/E15-app-config.md), [E17-testing](../foundations/E17-testing.md)   ·   **Related:** [F13-catalog-commands](F13-catalog-commands.md), [E01-architecture](../foundations/E01-architecture.md)

> Requirement tag: **CLI**

---

## 1. Purpose & Scope

One binary, several subcommands. You spawn `babel-lsp lsp` from your editor; you run `babel-lsp check` in CI; and you drive the pybabel catalog ops with `extract`, `update`, and `compile`. The `check` command runs the *same* diagnostics engine the editor runs — the parity test in [E17 REQ-TST-05](../foundations/E17-testing.md) keeps the two from ever drifting.

The headline is `check`. It scans your workspace once, runs every [F03](F03-diagnostics.md) check, and prints what it found in whichever format your tooling wants — concise lines for your terminal, JSON for a script, GitHub or GitLab annotations for your pipeline. The output contract mirrors ruff exactly, so anything that already consumes ruff output consumes this.

This spec covers:

- the subcommand surface — `lsp`, `check` (with `--fix`), `stats`, and the `extract`/`update`/`compile` passthroughs;
- the `check` output contract — nine `--output-format` values, byte-for-byte;
- the code filter (`--select`/`--ignore`), exit codes, the summary line, and color rules.

## 2. Non-Goals / Out of Scope

- **The pybabel ops themselves** — `extract`/`update`/`compile` behavior is owned by [F13-catalog-commands](F13-catalog-commands.md); this spec only wires them to the CLI.
- **The diagnostic catalog** — codes, severities, and firing conditions are [F03](F03-diagnostics.md)'s; `check` is a consumer.
- **Config resolution** — sources, precedence, and discovery are [E15](../foundations/E15-app-config.md)'s; `check` reads the resolved config.
- **Non-deterministic fixes** — `check --fix` (REQ-CLI-09) applies only the *deterministic* [F07](F07-code-actions.md) edits; ambiguous ones are left for a human, exactly as the editor leaves them.
- **Watch mode for `check`** — run it again; the editor is the watch mode.

## 3. Detailed Specification

### 3.1 `babel-lsp lsp` — serve the language server

This is the subcommand your editor spawns. It runs the long-lived server.

**REQ-CLI-01 — The server subcommand speaks stdio.**

`babel-lsp lsp` serves the language server over stdio — the one transport v1 ships ([E01 REQ-ARCH-01](../foundations/E01-architecture.md)).

```text
# stdio is the only transport — this is what an editor's config spawns
babel-lsp lsp
babel-lsp lsp --stdio
```

`--stdio` is implied when no flag is given, and the bare `babel-lsp` (no subcommand) is an alias for `babel-lsp lsp --stdio`, since several editors' default configs assume that shape. No remote transport ships in v1 — `--tcp` and `--http` are deferred ([E01](../foundations/E01-architecture.md) resolved OQ-ARCH-2 to stdio-only).

### 3.2 `babel-lsp check` — headless diagnostics

This is the linter. It runs every diagnostic over the workspace and prints findings, then exits with a code your CI can gate on.

**REQ-CLI-02 — `check` runs the full diagnostics pipeline once and reports.**

You pass zero or more paths; each is a file or directory. With no path, the current directory is the workspace. The run is exactly: scan → link → every [F03](F03-diagnostics.md) check → print → exit.

```text
# lint the whole workspace; default concise output
babel-lsp check

# lint just the German catalog and the views file
babel-lsp check locale/de/LC_MESSAGES/messages.po app/views.py
```

A path that is a single file still resolves config and links against the *enclosing* workspace (nearest `pyproject.toml`/`.git`), so cross-file checks like `po/duplicate-id` and `proj/unused-id` work; only findings located in the given paths are printed.

**REQ-CLI-03 — `check` and the LSP server share one diagnostics engine.**

`check` constructs the same `WorkspaceState` ([E07](../foundations/E07-data-model.md)), runs the same pass 1 / pass 2, and calls the same `run_checks` ([F03 §7](F03-diagnostics.md)). No check may exist in one mode and not the other. The e2e parity test asserts this on the broken shopfront fixtures ([E17 REQ-TST-05](../foundations/E17-testing.md)) — it compares the CLI's findings (code, file, range) against what the server publishes for the same workspace. This is the command [F16](F16-release-ci.md) wires into CI and pre-commit.

### 3.3 The code filter

You scope which checks run. The flags mirror the config keys exactly ([E15 REQ-CFG-05](../foundations/E15-app-config.md)) so a `pyproject.toml` rule and a CLI flag mean the same thing.

**REQ-CLI-04 — `--select` and `--ignore` mirror the diagnostics config.**

`--select` names the codes (or `all`) to enable; `--ignore` names codes to drop, applied after `select` — identical resolution to the config. CLI flags override the resolved config. An unknown code is a config error (exit 2), not a silent skip, so a typo can't quietly disable a check.

```text
# run only the placeholder and plural checks
babel-lsp check --select po/format-mismatch,po/plural-count

# run everything except the fuzzy noise
babel-lsp check --ignore po/fuzzy,po/missing-translation
```

Codes are parsed by `DiagCode::parse` ([F03 §7](F03-diagnostics.md)) — the diagnostics enum is the single source of valid spellings, shared by config and CLI alike.

### 3.4 `babel-lsp extract | update | compile` — catalog ops

These wrap the pybabel workflow so you never leave the binary. They are editor-agnostic and reuse the shared command layer from [F13](F13-catalog-commands.md).

**REQ-CLI-05 — The catalog subcommands invoke the shared pybabel ops.**

`extract` regenerates the `.pot` template, `update` merges it into each locale's `.po`, and `compile` builds `.mo` binaries. Each is a thin CLI front over the same routine [F13](F13-catalog-commands.md) exposes to the LSP's `workspace/executeCommand`, so the editor button and the terminal run identical logic.

```text
# regenerate locale/messages.pot from the source tree
babel-lsp extract

# merge the template into every locale's .po, then build .mo files
babel-lsp update
babel-lsp compile
```

Arguments, the `pybabel` binary discovery (`pybabel_path`, [E15](../foundations/E15-app-config.md)), and failure behavior are [F13](F13-catalog-commands.md)'s; this spec only declares that the subcommands exist and delegate.

### 3.5 The `check` output contract

You pick an output shape with `--output-format`; the default is `concise`. The shapes match ruff's so existing pipeline integrations work unchanged. Each value is a stable, documented format.

**REQ-CLI-06 — `--output-format` selects one of nine renderers.**

| Format | Shape | Best for |
|---|---|---|
| `concise` | One line per finding: `path:line:col: CODE message` | The terminal, quick scans (default) |
| `full` | A source-snippet block per finding, with caret underline and a `help:` line | Reading a finding in context |
| `json` | A pretty-printed JSON array of finding objects | Scripts, dashboards |
| `json-lines` | NDJSON — one finding object per line | Streaming, large result sets |
| `github` | GitHub Actions workflow annotations | GitHub CI |
| `gitlab` | GitLab Code Quality report JSON | GitLab CI merge-request widgets |
| `junit` | JUnit XML — each finding a `<testcase>` failure | Generic CI test-report ingestion |
| `grouped` | Findings grouped under a per-file header, indented | Reading many findings across files |
| `pylint` | One line per finding: `path:line: [CODE] message` | Pylint-compatible tooling |

`concise`, `full`, `grouped`, and `pylint` are human-facing; `json`, `json-lines`, `github`, `gitlab`, and `junit` are machine-facing. All nine are computed from the same `Finding` set — the format is purely a renderer.

**REQ-CLI-07 — Exit codes gate the build.**

| Exit | Meaning |
|---|---|
| `0` | Clean — no findings (or `--exit-zero` forced it) |
| `1` | One or more findings present |
| `2` | Fatal — usage error, unknown code, or unreadable config |

Exit `1` fires on *any* finding regardless of severity, matching ruff: a Hint counts. Use `--select`/`--ignore` to scope which codes can fail the build. `--exit-zero` forces exit `0` even with findings — for a reporting run that must not break the pipeline. Exit `2` is reserved for the CLI itself failing, never for findings.

**REQ-CLI-08 — The summary line and color.**

After the findings, `check` prints a one-line summary: `Found N errors.` when any finding printed, `All checks passed!` when none did. ("errors" follows ruff's wording for the finding count, not the severity.) Color follows severity — error red, warning yellow, info blue — and the `help:` lines in `full` format are cyan. Color is emitted only to a TTY and is suppressed when `NO_COLOR` is set or `--output-format` is a machine format. The summary line is omitted for the machine formats (`json`, `json-lines`, `github`, `gitlab`, `junit`).

### 3.6 `babel-lsp check --fix` — apply the deterministic fixes

`--fix` turns the linter into a fixer: it runs the checks, applies the safe fixes to disk, and re-reports what remains.

**REQ-CLI-09 — `--fix` applies the deterministic F07 fixes to disk.**

`check --fix` runs the normal pass, then applies each finding's paired [F07](F07-code-actions.md) edit that is *deterministic* — provably correct with no human choice (P4): copy a msgid into an empty `msgstr`, sync a placeholder, drop an obsolete entry. Ambiguous fixes are skipped and still reported. The edits are written to the files directly, so the F07 action layer must produce a `WorkspaceEdit` independent of any editor and the CLI applies it to the filesystem — a v1 requirement, and the reason `check` and the editor share one fix layer. By default the exit code follows the *remaining* findings; `--exit-non-zero-on-fix` forces exit `1` whenever anything was fixed, for a CI gate that should stay red until the tree is clean.

### 3.7 `babel-lsp stats` — translation coverage

`stats` answers "how translated is this project?" without opening a catalog.

**REQ-CLI-10 — `stats` reports per-locale coverage.**

`babel-lsp stats` prints a table — per locale: total messages, translated count and percent, fuzzy count, missing count — read straight from the index (`all_locales`/`missing_locales`, [E07 REQ-IDX-04](../foundations/E07-data-model.md)). It is the headless twin of the F12 coverage lens. `--output-format json` emits the same numbers for a dashboard, and a `--min-coverage <pct>` gate that exits `1` below a threshold is a natural follow-up. The cost is near zero — the index already holds every number.

```text
# coverage across every locale
babel-lsp stats

Locale   Messages   Translated   Fuzzy   Missing
de            142    138 (97%)        3         1
fr            142     90 (63%)        0        52
```

## 4. Data Shapes

The `json` and `json-lines` formats serialize each `Finding` ([F03 §7](F03-diagnostics.md)) to this object. Rows and columns are **1-based**, matching ruff; `end_location` is exclusive. `fix` carries the deterministic [F07](F07-code-actions.md) edit when one exists (the same edit `--fix` applies, REQ-CLI-09), or `null` when the finding has no automatic fix.

```json
{
  "code": "po/format-mismatch",
  "message": "placeholder '%(num)d' missing from translation",
  "location": { "row": 14, "column": 9 },
  "end_location": { "row": 14, "column": 17 },
  "filename": "locale/de/LC_MESSAGES/messages.po",
  "severity": "warning",
  "url": "https://babel-lsp.dev/rules/po-format-mismatch",
  "fix": null
}
```

`severity` is the resolved level after config overrides (`error` · `warning` · `info` · `hint`). `url` links the rule's documentation. The Rust surface, a thin layer over [F03](F03-diagnostics.md)'s engine:

```rust
// src/cli/mod.rs — clap derive
pub enum Cli {
    Lsp(LspArgs),
    Check(CheckArgs),
    Extract(CatalogArgs), Update(CatalogArgs), Compile(CatalogArgs),
}

pub struct LspArgs   { pub stdio: bool }
pub struct CheckArgs {
    pub paths: Vec<PathBuf>,
    pub select: Vec<DiagCode>,        // mirrors diagnostics.select
    pub ignore: Vec<DiagCode>,        // mirrors diagnostics.ignore
    pub output_format: OutputFormat,
    pub exit_zero: bool,
}

pub enum OutputFormat {
    Concise, Full, Json, JsonLines, Github, Gitlab, Junit, Grouped, Pylint,
}

// src/cli/check.rs
pub fn run_check(args: CheckArgs) -> ExitCode;   // 0 clean · 1 findings · 2 fatal
pub enum CliError { BadCode(String), BadConfig(PathBuf), Io(std::io::Error) }  // all → exit 2
```

Files: `cli/mod.rs` (parse + dispatch), `cli/check.rs` (one-shot pipeline), `cli/format.rs` (the nine renderers). The catalog subcommands delegate to [F13](F13-catalog-commands.md)'s command layer.

## 5. Examples & Use Cases

The findings below are the same broken shopfront from [F03 §8](F03-diagnostics.md): a dropped placeholder in the German catalog and a typo'd msgid in the views file. Each format renders that same finding set.

`concise` (default) — one grep-friendly line each. The comment names the format:

```text
# babel-lsp check
locale/de/LC_MESSAGES/messages.po:14:9: po/format-mismatch placeholder '%(num)d' missing from translation
app/views.py:21:12: msg/unknown-id msgid 'Chekout' is in no catalog or template
Found 2 errors.
```

`full` — a source snippet per finding, with a `-->` pointer, a numbered gutter, `^^^` carets under the offending span, and a cyan `help:` line:

```text
# babel-lsp check --output-format full
po/format-mismatch: placeholder '%(num)d' missing from translation
  --> locale/de/LC_MESSAGES/messages.po:14:9
   |
13 | msgid "%(num)d items in your cart"
14 | msgstr "%(naam)d Artikel in Ihrem Warenkorb"
   |         ^^^^^^^^ '%(num)d' missing; '%(naam)d' is extra
   |
help: keep the msgid's placeholders byte-identical in the translation

Found 1 error.
```

`json` — a pretty array; `--output-format json-lines` prints these objects one per line, unindented, with no wrapping array and no summary:

```json
[
  {
    "code": "po/format-mismatch",
    "message": "placeholder '%(num)d' missing from translation",
    "location": { "row": 14, "column": 9 },
    "end_location": { "row": 14, "column": 17 },
    "filename": "locale/de/LC_MESSAGES/messages.po",
    "severity": "warning",
    "url": "https://babel-lsp.dev/rules/po-format-mismatch",
    "fix": null
  }
]
```

`github` — one workflow-command annotation per finding; GitHub renders these inline on the PR diff:

```text
# babel-lsp check --output-format github
::error title=babel-lsp (po/format-mismatch),file=locale/de/LC_MESSAGES/messages.po,line=14,col=9::placeholder '%(num)d' missing from translation
```

`gitlab` — a Code Quality JSON array; the `fingerprint` is a stable hash so GitLab tracks the finding across pushes:

```json
[
  {
    "check_name": "po/format-mismatch",
    "description": "placeholder '%(num)d' missing from translation",
    "severity": "minor",
    "fingerprint": "b1946ac92492d2347c6235b4d2611184",
    "location": {
      "path": "locale/de/LC_MESSAGES/messages.po",
      "lines": { "begin": 14 }
    }
  }
]
```

`pylint` — the line-oriented pylint shape, for tools that already parse it:

```text
# babel-lsp check --output-format pylint
locale/de/LC_MESSAGES/messages.po:14: [po/format-mismatch] placeholder '%(num)d' missing from translation
```

A clean run prints only the summary (concise/full/grouped/pylint); CI gets exit `0`:

```text
# babel-lsp check
All checks passed!
```

In CI, [F16](F16-release-ci.md) runs `babel-lsp check --output-format github` so findings annotate the PR; a pre-commit hook runs plain `babel-lsp check` and blocks the commit on exit `1`.

## 6. Edge Cases & Failure Modes

- A path inside a larger project → the *workspace* is the enclosing project root, so cross-file linking works; only findings under the given paths print.
- Unknown code in `--select`/`--ignore` → exit 2 with the list of valid codes; a silent skip would hide checks.
- `--select` and `--ignore` naming the same code → `ignore` wins (it applies after `select`), matching the config resolution — not an error.
- No catalogs and no recognizable locale directory under the paths → exit 0, `All checks passed!`, with a `no catalogs found` note on stderr.
- `--exit-zero` with findings → findings still print in full; only the exit code is forced to 0.
- `NO_COLOR` set, or output piped to a non-TTY → color is suppressed; the text is otherwise identical.
- A machine format (`json` … `junit`) with zero findings → an empty but well-formed document (`[]`, no annotations, an empty JUnit suite), never the summary line.

## 7. Open Questions & Decisions

- **Decision (resolves OQ-CLI-1)** — `check --fix` ships in v1 (REQ-CLI-09), applying the deterministic [F07](F07-code-actions.md) fixes to disk. This makes the F07 action layer's editor-independence a v1 requirement rather than a deferred refactor, and populates the JSON `fix` field for every fixable finding.
- **Decision (resolves OQ-CLI-2)** — `babel-lsp stats` ships in v1 (REQ-CLI-10): a per-locale translation-coverage report, the headless twin of the F12 lens, at near-zero cost over the index. A `--min-coverage` CI gate is recorded as a follow-up, not v1.
- **Decision** — Findings are filtered by code, not family, matching [E15 REQ-CFG-05](../foundations/E15-app-config.md); a `po/*` glob is a possible later convenience.
- **Decision** — `check` exits `1` on *any* finding severity (ruff semantics), not only Warning/Error; scope with `--select`/`--ignore` rather than relying on severity to gate.

## 8. Cross-References

- **Depends on:** [F03-diagnostics](F03-diagnostics.md) — the `Finding`/`DiagCode` shapes and the `run_checks` engine `check` reuses; [E15-app-config](../foundations/E15-app-config.md) — `--select`/`--ignore` mirror `diagnostics.select`/`ignore`; [E17-testing](../foundations/E17-testing.md) — REQ-TST-05 CLI/server parity.
- **Related:** [F13-catalog-commands](F13-catalog-commands.md) — the `extract`/`update`/`compile` ops the subcommands delegate to; [E01-architecture](../foundations/E01-architecture.md) — the shared scan/link pipeline; [E07-data-model](../foundations/E07-data-model.md) — `WorkspaceState` and the index queries; [F16](F16-release-ci.md) — runs `check` in CI and pre-commit.

## 9. Changelog

- **2026-06-15** — Resolved the CLI open questions: stdio-only transport, dropping `--tcp`/`--http` (REQ-CLI-01, E01 OQ-ARCH-2); `check --fix` ships in v1 applying deterministic F07 fixes (REQ-CLI-09, OQ-CLI-1); the `babel-lsp stats` coverage report ships (REQ-CLI-10, OQ-CLI-2).
- **2026-06-15** — Initial draft: the `lsp`/`check`/`extract`/`update`/`compile` subcommand surface; the `check` output contract with nine ruff-style `--output-format` renderers and the 1-based JSON finding shape; `--select`/`--ignore` config parity; exit codes (0/1/2, `--exit-zero`); the summary line and NO_COLOR rules; OQ-CLI-2 a `stats` coverage subcommand.
