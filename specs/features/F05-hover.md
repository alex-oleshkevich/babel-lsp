# F05 — Hover

> **Status:** Draft
>
> **Version:** 0.2   ·   **Last updated:** 2026-06-15
>
> **Purpose:** A markdown translation table shown when you hover a msgid — every locale, its domain, and its status.
>
> **Depends on:** [F01-catalog-index](F01-catalog-index.md), [F02-message-extraction](F02-message-extraction.md)   ·   **Related:** [F04-completion](F04-completion.md), [E07-data-model](../foundations/E07-data-model.md)

> Requirement tag: **HOV**

---

## 1. Purpose & Scope

Hover a msgid and the server shows you every translation it has — one row per locale, with its domain and status.

You hover `_("Checkout")` in `views.py` and a card appears. It names the message, then lays out a table: German says `Kasse` and is `ok`, French is still `missing`. You see the state of every catalog without opening one.

This spec covers:

- Resolving the cursor to a msgid, on a source translation call or a catalog entry.
- Composing the hover card: id, context, plural, then the per-locale translation table.
- Reading [CatalogEntry](../foundations/E07-data-model.md) data through [CatalogIndex.lookup](../foundations/E07-data-model.md).
- The "no translations found" case and the unresolved-msgid silence.

## 2. Non-Goals / Out of Scope

- Building or loading the index — owned by [F01](F01-catalog-index.md).
- Extracting the translation call and its `msgid_range` — owned by [F02](F02-message-extraction.md).
- Judging an entry wrong (placeholder mismatch, missing plural form) — owned by [F03](F03-diagnostics.md). Hover *reports* status; it does not diagnose.

## 3. Detailed Specification

### 3.1 Dispatch

Hover resolves the cursor to a msgid from one of two sources, then renders the same card.

**REQ-HOV-01 — One provider, two hover targets.**

The provider takes the position and looks for a msgid under it. On a source file, it finds the [TranslationCall](../foundations/E07-data-model.md) whose `msgid_range` contains the cursor. On a `.po`/`.pot` buffer, it finds the catalog entry whose msgid line the cursor sits on. Either way it ends with a [CatalogKey](../foundations/E07-data-model.md) — `(msgid, msgctxt)` — to look up. No msgid under the cursor returns null quickly; most hovers in a file are not ours.

```rust
// src/features/hover.rs
pub fn hover(state: &WorkspaceState, uri: &Uri, pos: Position) -> Option<Hover>;
```

### 3.2 The translation table

The card names the message, then tables its translations across every locale and domain.

**REQ-HOV-02 — Render id, context, plural, then a table.**

The card is markdown. It opens with the msgid. It adds a context line only when the call was `pgettext` (the key carries an `msgctxt`), and a plural line only when `msgid_plural` is present. Then it renders one table row per [CatalogEntry](../foundations/E07-data-model.md) that [lookup](../foundations/E07-data-model.md) returns for the key, sorted by locale then domain. Each row shows the locale, the domain, the translation, and the status.

**REQ-HOV-03 — Status is one of ok, fuzzy, or missing.**

Each entry's status is read, never guessed (P5). An entry flagged `fuzzy` is `fuzzy`. An entry with a non-empty `msgstr` is `ok`. An entry whose `msgstr` is empty is `missing`, and its translation cell renders as an em dash `—`. A long translation is truncated for the cell so the card stays readable.

**REQ-HOV-04 — No entries means "no translations found".**

When `lookup` returns nothing, the msgid is in no catalog. The card still names the message, then says *No translations found* instead of a table. This is the typo case — `_("Chekout")` resolves to no key — and the card tells you so rather than showing an empty grid.

### 3.3 The anchor

**REQ-HOV-05 — The hover anchors to the msgid, not the whole call.**

The returned hover's range is the `msgid_range` — the string literal alone ([E07 REQ-IDX-06](../foundations/E07-data-model.md)). The editor underlines just the msgid, so the highlight matches what the card is about.

## 4. Examples & Use Cases

You hover `_("Checkout")` in the shopfront's `views.py`. The provider finds the call, resolves the key `(Checkout, None)`, and calls `lookup`. Two entries come back — German `Kasse`, French empty — and the card renders:

```markdown
**msgid** `Checkout`

| Locale | Domain | Translation | Status |
|--------|--------|-------------|--------|
| de | messages | Kasse | ok |
| fr | messages | — | missing |
```

German is translated; French still needs work — and you saw it without leaving the source line. As your editor renders that card, anchored under the msgid:

```text
  _("Checkout")
    ╰────────╯
  ╭─ babel-lsp ──────────────────────────────────╮
  │ msgid  Checkout                              │
  │                                              │
  │ Locale   Domain     Status     Translation   │
  │ de       messages   ✓ ok        Kasse        │
  │ fr       messages   ⚠ missing    —           │
  ╰──────────────────────────────────────────────╯
```

Now you hover `pgettext("button", "Save")`. The key carries a context, and German marked this one fuzzy:

```markdown
**msgid** `Save`

**context** `button`

| Locale | Domain | Translation | Status |
|--------|--------|-------------|--------|
| de | messages | Speichern | fuzzy |
| fr | messages | — | missing |
```

The fuzzy German row warns you the translation exists but is unverified — gettext ignores it at runtime, and now so do you. The card carries the context line above the table:

```text
  pgettext("button", "Save")
                     ╰────╯
  ╭─ babel-lsp ──────────────────────────────────╮
  │ msgid    Save                                │
  │ context  button                              │
  │                                              │
  │ Locale   Domain     Status     Translation   │
  │ de       messages   ~ fuzzy     Speichern    │
  │ fr       messages   ⚠ missing    —           │
  ╰──────────────────────────────────────────────╯
```

Hover `ngettext("%(num)d item", "%(num)d items", n)` and the card adds the plural line:

```markdown
**msgid** `%(num)d item`

**plural** `%(num)d items`
```

## 5. Edge Cases & Failure Modes

- Cursor on a non-literal msgid — `_(f"Hello {user}")` — → the call is [unresolved](../glossary.md), its `msgid` is `None`, so there is no key and no hover (P4).
- Cursor in the source call but outside the `msgid_range` (on the function name or a later arg) → null, not a card.
- A msgid defined in several domains — `messages` and `admin` → each domain gets its own rows, so one locale can appear twice, distinct per domain.
- A fuzzy entry → shown as `fuzzy`, never silently rendered as `ok`.
- An [obsolete](../glossary.md) entry → not shown; it no longer describes a live msgid.
- Hovering the msgid line inside a `.po` buffer → the same card, so you see sibling locales while editing one catalog.

## 6. Cross-References

- **Depends on:** [F01-catalog-index](F01-catalog-index.md) — supplies the index hover reads via `lookup`; [F02-message-extraction](F02-message-extraction.md) — supplies the `TranslationCall` and its `msgid_range`.
- **Related:** [E07-data-model](../foundations/E07-data-model.md) — `CatalogEntry`, `CatalogKey`, and the `lookup` API hover calls; [F04-completion](F04-completion.md) — the other reader of the index, sharing the locale/status vocabulary.
- **Testing:** [E17 §2.5](../foundations/E17-testing.md) — this feature's row in the e2e coverage matrix.

## 7. Changelog

- **2026-06-15** — v0.2: added ASCII mockups showing how the hover card renders in the editor for the plain and `pgettext` (context) cases, anchored under the msgid (per the constitution §6 editor-surface-mockup allowance).
- **2026-06-15** — Initial draft: one provider dispatching on source calls and catalog entries (REQ-HOV-01); the id/context/plural header plus the per-locale translation table (REQ-HOV-02/03); the "no translations found" case (REQ-HOV-04); the msgid-range anchor (REQ-HOV-05). Translated from the legacy `features/hover.rs`.
</content>
</invoke>
