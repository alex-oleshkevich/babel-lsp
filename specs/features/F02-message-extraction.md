# F02 — Message Extraction

> **Status:** Draft
>
> **Version:** 0.2   ·   **Last updated:** 2026-06-15
>
> **Purpose:** Source-side pass 1 — turning the gettext calls in your Python and Jinja into `TranslationCall` facts, error-tolerantly and with exact ranges, so every downstream feature reads messages instead of re-parsing files.
>
> **Depends on:** [E07-data-model](../foundations/E07-data-model.md), [E03-tech-stack](../foundations/E03-tech-stack.md)   ·   **Related:** [F01-catalog-index](F01-catalog-index.md), [F03-diagnostics](F03-diagnostics.md)

> Requirement tag: **EXT**

---

## 1. Purpose & Scope

This spec teaches the server to find translation calls in source. You write `_("Checkout")` in a view or `{{ _("Your cart") }}` in a template; this pass turns each into one fact the rest of the suite can link, validate, and navigate.

This spec covers:

- The gettext variants the server recognizes and their argument layouts
- Python extraction over tree-sitter — callee resolution, string args, ranges
- Jinja extraction over a tree-sitter Jinja grammar — expression calls and `{% trans %}` blocks
- Unresolved msgids — non-literal first arguments kept but not looked up
- Error tolerance — partial facts from broken source (P3)

## 2. Non-Goals / Out of Scope

- The `TranslationCall` and `TranslationFunc` shapes themselves — defined in [E07](../foundations/E07-data-model.md), referenced here by name.
- Catalog (`.po`/`.pot`) extraction — owned by [F01](F01-catalog-index.md); this spec is source-only.
- Diagnostics fired on extracted facts — `msg/non-constant-id`, `msg/fstring-in-call`, and the rest live in [F03](F03-diagnostics.md). This spec produces the facts those checks read; it does not own the checks.
- Pass-2 linking of msgids to catalog entries — owned by [E07](../foundations/E07-data-model.md) and [F01](F01-catalog-index.md).

## 3. Background & Rationale

A translation call is the only place source and catalog meet. Find every call precisely and the whole server works; miss one and a real translation looks unknown.

The Python path was always tree-sitter, and it stays that way. The Jinja path was not. The legacy server matched `{{ _(...) }}` and `{% trans %}` blocks with regular expressions, and the constitution records why that is replaced: regex "worked for the common case but broke on nested braces, escaped quotes, and multi-line blocks" (constitution §3, *Rejected: regex-based Jinja extraction*). This spec is that replacement — Jinja now parses through the in-house [`tree-sitter-jinja2`](https://github.com/alex-oleshkevich/tree-sitter-jinja2) grammar ([E03](../foundations/E03-tech-stack.md)), giving the same error-tolerant, position-accurate extraction the Python path already enjoys.

## 4. Concepts & Definitions

The vocabulary is canonical in the [glossary](../glossary.md): a **translation call** is the source-side fact, a **gettext variant** is one recognized function with a known argument layout, an **unresolved msgid** is a call whose first argument is not a literal, and **msgctxt** and **domain** are the disambiguating context and text domain. This spec links those terms; it does not redefine them.

## 5. Detailed Specification

### 5.1 The recognized variants

The server recognizes one fixed family of gettext functions, plus aliases and configured extras. Each variant fixes where the msgid, plural, context, and domain sit in the argument list.

**REQ-EXT-01 — The variant table is the argument-layout contract.**

A callee name maps to a [`TranslationFunc`](../foundations/E07-data-model.md) and, through it, to the positions the extractor reads. The string-argument positions, in source order:

| Function | `TranslationFunc` | Arg positions (strings only) |
|---|---|---|
| `_`, `gettext` | `Gettext` | `(msgid)` |
| `ngettext` | `NGettext` | `(msgid, msgid_plural, n)` |
| `pgettext` | `PGettext` | `(msgctxt, msgid)` |
| `npgettext` | `NPGettext` | `(msgctxt, msgid, msgid_plural, n)` |
| `dgettext` | `DGettext` | `(domain, msgid)` |
| `dngettext` | `DNGettext` | `(domain, msgid, msgid_plural, n)` |
| `dpgettext` | `DPGettext` | `(domain, msgctxt, msgid)` |
| `dnpgettext` | `DNPGettext` | `(domain, msgctxt, msgid, msgid_plural, n)` |

The numeric `n` argument is never a string, so it is read positionally but never stored as a message. Order within the layout is fixed: domain first, then context, then msgid, then plural — `dnpgettext` carries all four. This is the `has_domain`/`has_context`/`has_plural` logic on `TranslationFunc` in [E07](../foundations/E07-data-model.md).

**REQ-EXT-02 — Aliases and lazy/u-forms collapse onto the base variant.**

Babel and Django ship aliases that mean the same thing. The server folds them onto the eight base variants when it resolves the callee name:

```rust
// src/extract/types.rs — name → variant
"_" | "gettext" | "gettext_lazy" | "ugettext" | "ugettext_lazy"  => Gettext,
"ngettext" | "ngettext_lazy" | "ungettext" | "ungettext_lazy"    => NGettext,
"pgettext" | "pgettext_lazy"                                     => PGettext,
// dgettext, dngettext, dpgettext, dnpgettext … unchanged
```

The `_lazy` suffix (deferred translation) and the legacy `u*` prefix (Python-2 unicode forms) change runtime behavior, not the message, so they share their base variant's layout.

**REQ-EXT-03 — `extra_keywords` extends the table from config.**

A project can name its own translation functions. The `[tool.babel-lsp]` `extra_keywords` setting maps a custom name to an existing variant — `{ "tr" = "gettext" }` makes `tr("Checkout")` a recognized `Gettext` call. Unknown names without a mapping are not calls and produce no fact.

### 5.2 Python extraction

You write translation calls as plain function calls; the extractor walks the parse tree for them. It uses `tree-sitter-python` ([E03](../foundations/E03-tech-stack.md)), visits every `call` node, and decides whether the callee is a recognized variant.

**REQ-EXT-04 — Resolve the callee, including attribute access.**

A call's `function` field is either an `identifier` (`gettext(...)`) or an `attribute` (`gettext.gettext(...)`, `translator.gettext(...)`). For an attribute, the server reads the rightmost name only — `gettext.gettext` resolves on `gettext`, the trailing attribute. A bare name resolves directly. Names that match no variant and no `extra_keywords` entry yield nothing, so `my_gettext("x")` is correctly ignored.

**REQ-EXT-05 — Read string-literal arguments by position.**

The extractor collects the call's string-literal arguments in order, then assigns them to slots per the REQ-EXT-01 layout for the resolved variant. A plain `string` node yields its unquoted content; the quote-stripping rejects f-strings and byte strings and tolerates `r`/`u` prefixes and triple quotes.

```rust
// src/extract/python.rs — argument slotting follows the variant
let domain   = func.has_domain().then(|| take_next_string());
let msgctxt  = func.has_context().then(|| take_next_string());
let msgid    = take_next_string();           // may be None → unresolved
let plural   = func.has_plural().then(|| take_next_string());
```

**REQ-EXT-06 — Concatenate adjacent string literals.**

Python joins adjacent string literals at compile time, and so does the extractor. `_("Order " "summary")` and the multi-line `concatenated_string` form both yield the single msgid `Order summary`. The `msgid_range` spans the whole concatenation.

**REQ-EXT-07 — Emit a `TranslationCall` with two ranges.**

Each recognized call becomes one [`TranslationCall`](../foundations/E07-data-model.md). Its `range` is the whole call expression — the anchor for "this is a translation call here". Its `msgid_range` is the msgid literal alone, so hover, goto, and rename land on the message text, not the parentheses. The shopfront `pgettext("button", "Save")` yields `func: PGettext`, `msgctxt: Some("button")`, `msgid: Some("Save")`, and a `msgid_range` covering only `"Save"`.

### 5.3 Jinja extraction

Templates call translations two ways, and both now parse through a tree-sitter Jinja grammar rather than regex. You write `{{ _("Your cart") }}` for an inline message and `{% trans %}…{% endtrans %}` for a block.

**REQ-EXT-08 — Expression calls parse like Python calls.**

Inside a `{{ … }}` output expression, a call to a recognized variant is extracted exactly as in §5.2 — same callee resolution, same positional slotting. The checkout template's `{{ _("Your cart") }}` yields a `Gettext` call with msgid `Your cart`, its `range` over the call and `msgid_range` over `"Your cart"`.

**REQ-EXT-09 — `{% trans %}` blocks are message facts.**

A `{% trans %}One item{% endtrans %}` block is a translation call whose msgid is the block body. The grammar gives the body as a node, so escaped characters and nested expressions stay intact — the failure mode regex had. The block's `range` spans `{% trans %}` through `{% endtrans %}`; the `msgid_range` covers the body text.

**REQ-EXT-10 — `{% pluralize %}` splits a block into singular and plural.**

A `{% pluralize %}` marker inside a trans block makes it a plural call. The text before the marker is the msgid, the text after is the `msgid_plural`, and the variant becomes `NGettext`. The shopfront checkout block is the canonical case:

```jinja
{# app/templates/checkout.html #}
{% trans count=n %}One item{% pluralize %}{{ count }} items{% endtrans %}
```

This yields `func: NGettext`, `msgid: "One item"`, `msgid_plural: "{{ count }} items"`.

**REQ-EXT-11 — Capture trans bindings and context.**

The `{% trans count=n %}` head binds template variables for the body; `count=n` is the count expression a plural block pluralizes on. The server records the binding names but does not evaluate them (P1 — nothing runs). A `{% trans context "button" %}` head sets the `msgctxt`, exactly like `pgettext`'s first argument.

### 5.4 Unresolved msgids

Sometimes the first argument is not a literal — a variable, an f-string, a concatenation of names. The server keeps the call but cannot know its msgid.

**REQ-EXT-12 — A non-literal msgid produces `msgid: None`.**

When the slot that should hold the msgid is not a resolvable string literal, the extractor still emits a `TranslationCall`, with `msgid: None` and `msgid_range: None`. This is the unresolved-msgid state from the [glossary](../glossary.md) and constitution P4: the call is kept for reporting but excluded from every catalog lookup, never guessed.

The f-string `_(f"Hello {user}")` and the variable `_(label)` both land here. These facts feed the [F03](F03-diagnostics.md) checks `msg/non-constant-id` and `msg/fstring-in-call` — F03 owns when those fire; F02 only guarantees the fact exists to fire on.

### 5.5 Error tolerance

Users edit mid-keystroke, so a template or module is often half-broken. The extractor must still return what it can.

**REQ-EXT-13 — Walk through ERROR nodes; return partial facts.**

Extraction never aborts on a parse error. tree-sitter produces a tree with `ERROR` nodes around the broken span; the walk descends into and past them, extracting every well-formed call it can still reach (P3). A missing closing paren on one call does not hide the three valid calls around it. No extractor returns an error type — the absence of a fact, or an unresolved fact, is the only failure signal.

## 6. Visualizations

The two source languages converge on one fact stream that pass 2 then links.

```mermaid
flowchart TB
    classDef src fill:#CCE5FF,stroke:#4A90D9,color:#004085
    classDef parse fill:#E2D4F0,stroke:#8E5EA8,color:#3D2952
    classDef fact fill:#D4EDDA,stroke:#28A745,color:#155724
    classDef unres fill:#FFF3CD,stroke:#FFC107,color:#333

    py["views.py\n_(\"Checkout\")"]:::src
    jinja["checkout.html\n{% trans %}…{% endtrans %}"]:::src
    tsp["tree-sitter-python\nwalk call nodes"]:::parse
    tsj["tree-sitter Jinja\nwalk trans / output"]:::parse
    call["TranslationCall\nmsgid + ranges"]:::fact
    none["TranslationCall\nmsgid: None"]:::unres

    py -->|"parse"| tsp
    jinja -->|"parse"| tsj
    tsp -->|"literal first arg"| call
    tsj -->|"literal first arg"| call
    tsp -->|"non-literal arg"| none

    linkStyle 0 stroke:#8E5EA8,stroke-width:2px
    linkStyle 1 stroke:#8E5EA8,stroke-width:2px
    linkStyle 2 stroke:#28A745,stroke-width:2px
    linkStyle 3 stroke:#28A745,stroke-width:2px
    linkStyle 4 stroke:#FFC107,stroke-width:2px
```

## 7. Examples & Use Cases

You open the shopfront. In `app/views.py`, three calls extract cleanly: `_("Checkout")` → `Gettext`/msgid `Checkout`; `pgettext("button", "Save")` → `PGettext`/msgctxt `button`/msgid `Save`; and `ngettext("%(num)d item", "%(num)d items", n)` → `NGettext` with msgid `%(num)d item` and plural `%(num)d items` — the `n` argument read but not stored.

In `app/templates/checkout.html`, `{{ _("Your cart") }}` extracts as a `Gettext` call, and the `{% trans count=n %}One item{% pluralize %}{{ count }} items{% endtrans %}` block extracts as an `NGettext` call with both forms (REQ-EXT-10). Five facts, two files — every one carrying the ranges that goto and hover anchor to.

Then someone writes `_(f"Hello {user}")`. The extractor still emits a call, but with `msgid: None` (REQ-EXT-12). It appears in no catalog lookup, and [F03](F03-diagnostics.md) raises `msg/fstring-in-call` on it.

## 8. Edge Cases & Failure Modes

- Non-literal first argument (`_(label)`, `_(f"…")`) → `msgid: None`, kept for [F03](F03-diagnostics.md), skipped by lookups.
- Attribute callee (`gettext.gettext("Checkout")`) → recognized on the trailing name (REQ-EXT-04).
- Look-alike name (`my_gettext("x")`) → not a variant, no fact; add via `extra_keywords` to recognize.
- Adjacent literals (`_("Order " "summary")`) → one msgid `Order summary` (REQ-EXT-06).
- Unterminated call or template tag → ERROR node walked past, surrounding calls still extracted (REQ-EXT-13).
- `{% trans %}` block with an empty body → emitted with msgid `""`; the empty-msgid judgement is [F03](F03-diagnostics.md)'s, not this pass's.
- Too few string arguments for the variant (`pgettext("button")` with no msgid) → msgid slot is empty → `msgid: None`, an unresolved fact rather than a dropped one.

## 9. Open Questions & Decisions

- **Decision (resolves OQ-EXT-1)** — `{% trans %}` placeholder bindings are extracted as **structured placeholders**. The body's `{{ count }}` references normalize to Babel's printf form `%(count)s` in the msgid — exactly what `pybabel extract` writes — and the placeholder set is recorded on the `TranslationCall`. So [F03](F03-diagnostics.md)'s `po/format-mismatch` and `po/extra-variable` validate a template message's translations the same way they validate a Python one: a German `msgstr` that drops `%(count)s` is flagged.
- **Decision (resolves OQ-EXT-2)** — A non-literal first argument is always unresolved in v1 — a module-level constant like `MSG = "Checkout"; _(MSG)` included. The call carries `msgid: None`, takes no lookup, and raises `msg/non-constant-id` at Information (P4: no guessing). Resolving simple module-level string constants is a possible later enhancement; it adds binding resolution and the risk of mis-resolving reassigned or imported names, so it stays out of v1.
- **Decision** — Jinja parses through tree-sitter, not regex; the trade is recorded in the constitution and not re-litigated here. The grammar is the in-house [`tree-sitter-jinja2`](https://github.com/alex-oleshkevich/tree-sitter-jinja2) ([E03](../foundations/E03-tech-stack.md), resolving OQ-TECH-1); the extraction contract stays grammar-agnostic, so the choice remains swappable.

## 10. Cross-References

- **Depends on:** [E07-data-model](../foundations/E07-data-model.md) — `TranslationCall`, `TranslationFunc`, and the `range`/`msgid_range` contract this pass fills; [E03-tech-stack](../foundations/E03-tech-stack.md) — the tree-sitter Python and in-house `tree-sitter-jinja2` grammars.
- **Related:** [F01-catalog-index](F01-catalog-index.md) — the catalog side of pass 1 and the linking these facts feed; [F03-diagnostics](F03-diagnostics.md) — consumes unresolved facts for `msg/non-constant-id` and `msg/fstring-in-call`; [F11-hardcoded-strings](F11-hardcoded-strings.md) — reuses this extraction to tell translated strings from un-translated literals.
- **Testing:** [E17 §2.5](../foundations/E17-testing.md) — this feature's row in the e2e coverage matrix.

## 11. Changelog

- **2026-06-15** — v0.2: resolved the extraction open questions — `{% trans %}` placeholders are extracted as structured `%(name)s` placeholders for the F03 checks (OQ-EXT-1); non-literal msgids, module constants included, stay unresolved in v1 (OQ-EXT-2); pinned the Jinja grammar to `tree-sitter-jinja2` (OQ-TECH-1).
- **2026-06-15** — Initial draft: the eight gettext variants plus aliases/`_lazy`/`u*`/`extra_keywords` (REQ-EXT-01…03), Python extraction over tree-sitter with attribute callees and adjacent-literal joining (REQ-EXT-04…07), Jinja expression and `{% trans %}`/`{% pluralize %}` extraction over a tree-sitter grammar replacing the rejected regex path (REQ-EXT-08…11), unresolved-msgid handling (REQ-EXT-12), and ERROR-node tolerance (REQ-EXT-13).
