# E15 — App Config

> **Status:** Draft
>
> **Version:** 0.2   ·   **Last updated:** 2026-06-15
>
> **Purpose:** Where settings come from, how locale directories are discovered, and every config key the server reads — including per-rule diagnostic toggles.
>
> **Depends on:** [E01-architecture](E01-architecture.md), [constitution](../constitution.md)   ·   **Related:** [F01-catalog-index](../features/F01-catalog-index.md), [F03-diagnostics](../features/F03-diagnostics.md)

> Requirement tag: **CFG**

---

## 1. Purpose & Scope

This spec defines configuration: the files it's read from, the precedence between them, how the server finds your catalogs when you say nothing, and the full key reference. Diagnostics, completion, and inlay hints all read their settings from here.

## 2. Detailed Specification

### 2.1 Config sources and precedence

**REQ-CFG-01 — Three sources, highest wins, built-in defaults under all.**

Settings resolve from these sources, highest precedence first:

1. `pyproject.toml` → `[tool.babel-lsp]` — the preferred home; one file the whole project already has.
2. `babel-lsp.toml` → root-level keys — a dedicated file when you don't want config in `pyproject.toml`.
3. `babel.cfg` → `[babel-lsp]` section, plus `[jinja2: ...]` sections read for Jinja extensions — the fallback that reuses Babel's own mapping file.
4. Built-in defaults — under everything.

A change to any of these files re-runs resolution and triggers a relink ([E01 REQ-ARCH-12](E01-architecture.md)).

### 2.2 Locale discovery

**REQ-CFG-02 — Find catalogs automatically when unconfigured.**

When `locale_dirs` is empty, the server discovers catalogs itself so a typical project needs zero config. It searches for `locales/`, `locale/`, and `translations/` directories at the root and inside Python packages — a package is a directory with `__init__.py`, named from `pyproject.toml`'s `[project].name` or `[tool.poetry].name`. Nested package locale folders (`myapp/locale/`, `myapp/admin/locale/`) are discovered too. The discovered set merges with any explicit `locale_dirs`. The path-to-`(locale, domain)` rule lives in [F01](../features/F01-catalog-index.md).

**REQ-CFG-03 — Detect starlette-babel.**

When the project depends on `starlette-babel` (seen in `pyproject.toml` dependencies), the server notes it; this informs sensible defaults for keyword and directory conventions. Detection never changes behavior the user explicitly configured.

### 2.3 The key reference

**REQ-CFG-04 — The configuration keys.**

| Key | Type | Default | Purpose |
|---|---|---|---|
| `locale_dirs` | `string[]` | `[]` (auto-discover) | Extra locale directories to scan. |
| `default_locale` | `string` | `null` | The preferred locale for previews and ordering. |
| `domains` | `string[]` | `null` (all) | Restrict indexing to these text domains. |
| `extra_keywords` | `string[]` | `[]` | Extra translation function names beyond the built-ins. |
| `jinja_extensions` | `string[]` | `[".html", ".jinja2", ".j2"]` | File extensions treated as Jinja templates. |
| `detect_hardcoded_strings` | `bool` | `false` | Enable hardcoded-string diagnostics ([F11](../features/F11-hardcoded-strings.md)). |
| `inlay_hint_locale` | `string` | `null` | Locale whose translation is shown as an inlay hint ([F08](../features/F08-inlay-hints.md)). |
| `position_encoding` | `string` | `"utf-8"` | Preferred encoding to negotiate; falls back to UTF-16. |
| `diagnostics.select` | `string[]` | `["all"]` | Diagnostic codes (or `all`) to enable. |
| `diagnostics.ignore` | `string[]` | `[]` | Diagnostic codes to disable, applied after `select`. |
| `diagnostics.severity` | `map` | `{}` | Per-code severity override, e.g. `{ "po/fuzzy" = "warning" }`. |
| `unchanged.ignore` | `string[]` | `[]` | Exact msgids that `po/unchanged` treats as legitimately identical across languages ([F03 REQ-DIAG-10](../features/F03-diagnostics.md)). |
| `pybabel_path` | `string` | `null` (auto) | Path to the `pybabel` binary for catalog commands ([F13](../features/F13-catalog-commands.md)). |
| `log_level` | `string` | `null` | `error` … `trace`. |
| `log_file` | `string` | `null` (stderr) | Optional log file path. |

### 2.4 Per-rule diagnostic configuration

**REQ-CFG-05 — Diagnostics select, ignore, and severity, ruff-style.**

Each diagnostic in [F03](../features/F03-diagnostics.md) has a stable `area/short-name` code, and any of them can be toggled or re-leveled without touching code. Resolution: start from `select` (default `["all"]`), subtract `ignore`, then apply `severity` overrides to whatever survives. An unknown code in either list is a config warning, not a hard error — a typo shouldn't silence the server. This is the same model the `check` CLI exposes as `--select`/`--ignore` ([F15](../features/F15-cli.md)), reading from the same resolved config.

## 3. Examples & Use Cases

The shopfront's `pyproject.toml` turns off the fuzzy warning and previews German inline:

```toml
# pyproject.toml
[tool.babel-lsp]
default_locale = "de"
inlay_hint_locale = "de"
extra_keywords = ["_t", "lazy_gettext"]

[tool.babel-lsp.diagnostics]
ignore = ["po/fuzzy"]
severity = { "po/missing-translation" = "warning" }
```

## 4. Edge Cases & Failure Modes

- No config and no recognizable locale directory → the server runs, indexes nothing, and stays silent; opening a `.po` still works.
- Conflicting keys across sources → highest-precedence source wins per key, not per file (a `babel-lsp.toml` can override one `pyproject.toml` key while inheriting the rest).
- `domains` names a domain with no catalogs → no error; that domain is simply empty.

## 5. Cross-References

- **Depends on:** [E01-architecture](E01-architecture.md) — config files are watched and trigger relink; [constitution](../constitution.md) — P5.
- **Related:** [F01-catalog-index](../features/F01-catalog-index.md) — uses `locale_dirs`/`domains`; [F03-diagnostics](../features/F03-diagnostics.md) — the codes toggled here; [F15-cli](../features/F15-cli.md) — `--select`/`--ignore` parity.

## 6. Changelog

- **2026-06-15** — v0.2: added the `unchanged.ignore` config key backing the `po/unchanged` allowlist ([F03](../features/F03-diagnostics.md) OQ-DIAG-1).
- **2026-06-15** — Initial draft: three config sources with per-key precedence, automatic locale discovery, the full key reference, and the ruff-style per-rule diagnostic configuration.
