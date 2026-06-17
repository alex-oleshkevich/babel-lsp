# babel-lsp

Language server for Python [Babel](https://babel.pocoo.org/) i18n. Works with `.py`, Jinja templates, and `.po`/`.pot` catalog files.

## Features

- **Diagnostics** — 32 checks across source files, PO catalogs, and the whole project
- **Hover** — translation table for any `_("msgid")` call
- **Completion** — msgid completion in translation calls
- **Inlay hints** — inline translation previews next to `_()` calls
- **Code actions** — remove fuzzy flags, fill missing translations, rename msgid across all files
- **Go-to-definition / references / rename** — treat msgids as first-class symbols
- **`babel-lsp check`** — headless CLI linter, same rules as the LSP

### Diagnostics

| Code | What it catches |
|------|----------------|
| `msg/empty-id` | `_("")` — empty string is the catalog header sentinel |
| `msg/fstring-in-call` | `_(f"…")` — f-string is interpolated before gettext sees it |
| `msg/format-before-call` | `_("…" % x)` — format applied before gettext |
| `msg/implicit-concat` | `_("a" "b")` — implicit string concatenation |
| `msg/non-constant-id` | `_(variable)` — pybabel cannot extract non-literal ids |
| `msg/unknown-id` | msgid not found in any catalog or template |
| `msg/missing-in-locale` | msgid translated in some locales but empty in others |
| `po/header-missing` | missing or incomplete catalog header |
| `po/duplicate-id` | same msgid appears twice in one file |
| `po/obsolete` | `#~` entry absent from the `.pot` template |
| `po/missing-translation` | empty msgstr in a `.po` file |
| `po/fuzzy` | entry marked `#, fuzzy` |
| `po/blank` | msgstr is whitespace-only |
| `po/plural-count` | wrong number of plural forms vs. `nplurals` |
| `po/same-plurals` | all plural forms are identical |
| `po/format-mismatch` | printf/brace placeholder missing from translation |
| `po/extra-variable` | translation has extra placeholders not in source |
| `po/unchanged` | translation is identical to the source string |
| `po/newline-count` | `\n` count differs between source and translation |
| `po/whitespace-edges` | leading/trailing whitespace differs |
| `po/end-punctuation` | trailing punctuation differs |
| `po/double-space` | translation has double space not in source |
| `po/repeated-word` | consecutive repeated word in translation |
| `po/escape-mismatch` | backslash escape sequences differ |
| `po/bracket-count` | bracket count differs |
| `po/accelerator-mismatch` | `&` accelerator marker count differs |
| `po/xml-tag-mismatch` | XML/HTML tag structure differs |
| `po/url-changed` | URL from source is absent or path-altered |
| `po/number-mismatch` | numeric literals differ |
| `proj/inconsistent-translation` | same msgid translated differently in the same locale |
| `proj/missing-locale-file` | a locale has `.po` for some domains but not all |
| `proj/unused-id` | msgid exists in catalog but is never referenced in source |

## Installation

```bash
cargo install babel-lsp
```

## Configuration

Place a `babel-lsp.toml` (or `pyproject.toml` `[tool.babel-lsp]` section) at your project root.

```toml
# Directories to search for .po/.pot files (auto-detected if absent)
locale_dirs = ["locale"]

# Default locale for features that need one
default_locale = "en"

# Restrict to specific translation domains
# domains = ["messages", "admin"]

# Additional translation functions beyond the built-ins
# (_, gettext, ngettext, pgettext, npgettext, lazy_gettext, …)
extra_keywords = ["my_gettext"]

# Extensions treated as Jinja templates
jinja_extensions = [".html", ".jinja", ".jinja2", ".j2"]

# Locale to show in inlay hints (defaults to first alphabetical locale)
# inlay_hint_locale = "de"

# Path to pybabel (auto-discovered on PATH if absent)
# pybabel_path = "/path/to/pybabel"

[diagnostics]
# Run only these checks (default: all)
# select = ["po/fuzzy", "po/missing-translation"]

# Suppress specific checks
ignore = ["po/unchanged", "po/same-plurals"]

# Override severity per code
[diagnostics.severity]
"po/fuzzy" = "warning"
"proj/unused-id" = "hint"

[unchanged]
# msgids to skip for po/unchanged (e.g. single-word proper nouns)
ignore = ["OK", "PDF"]
```

## CLI

```bash
# Check the current project
babel-lsp check

# Check specific paths
babel-lsp check locale/ src/

# Run only selected checks
babel-lsp check --select po/fuzzy,po/missing-translation

# Suppress checks
babel-lsp check --ignore po/unchanged

# Auto-fix what can be fixed (fuzzy flags, missing translations)
babel-lsp check --fix

# Output formats: concise (default), full, json, json-lines, grouped,
#                 github, gitlab, junit, pylint
babel-lsp check --output-format github

# Exit 0 even with findings (useful in pre-commit hooks that only annotate)
babel-lsp check --exit-zero
```

## Editor Setup

### Zed

Install the bundled extension:

```bash
just install-zed
```

Then add to `~/.config/zed/settings.json`:

```json
{
  "languages": {
    "Python": { "language_servers": ["babel-lsp", "..."] }
  }
}
```

### Helix

Merge `editors/helix/languages.toml` into `~/.config/helix/languages.toml`.

### Neovim

Requires Neovim 0.11+. Add to your `init.lua`:

```lua
require("lspconfig").babel_lsp.setup {}
```

Or use the minimal snippet from `editors/neovim/babel_lsp.lua`.

### Other editors

The server speaks standard LSP over stdio:

```bash
babel-lsp lsp --stdio
```

## Fixtures

`fixtures/` contains one minimal workspace per diagnostic code. Use them to verify diagnostics work or as templates for your own test setups:

```bash
babel-lsp check fixtures/po-fuzzy/
babel-lsp check fixtures/po-fuzzy/ --select po/fuzzy
```
