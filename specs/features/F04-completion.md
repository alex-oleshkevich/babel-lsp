# F04 — Completion

> **Status:** Draft
>
> **Version:** 0.1   ·   **Last updated:** 2026-06-15
>
> **Purpose:** msgid autocompletion inside translation calls, with a multi-locale preview of each candidate.
>
> **Depends on:** [F01-catalog-index](F01-catalog-index.md), [F02-message-extraction](F02-message-extraction.md)   ·   **Related:** [F05-hover](F05-hover.md), [E07-data-model](../foundations/E07-data-model.md)

> Requirement tag: **CPL**

---

## 1. Purpose & Scope

When you start typing a msgid inside a translation call, this feature offers every msgid your catalogs already know, and previews how each one is translated across your locales.

You type `_("Che` in `views.py`. The server offers `Checkout`, and the completion shows `[de] Kasse` beside it — so you pick the real msgid without leaving the editor or guessing at its exact spelling.

This spec covers:

- The trigger: typing a quote inside a recognized translation call's msgid position.
- The candidates: every msgid in the [catalog index](F01-catalog-index.md), prefix-matched first.
- What each completion item shows — its detail line, its multi-locale table, and its precise text edit.
- Context-awareness: a `pgettext` call prefers msgids that carry its context.

## 2. Non-Goals / Out of Scope

- Building the index of msgids — owned by [F01](F01-catalog-index.md); this feature only reads `all_msgids`.
- Detecting which call the cursor sits in — the call shapes and ranges come from [F02](F02-message-extraction.md).
- Completing anything that isn't a msgid (kwargs, plurals, domain names) — Python's own LSP owns those.
- The hover surface that renders the same translations on a finished call — owned by [F05](F05-hover.md).

## 3. Detailed Specification

The provider runs in three steps: decide whether the cursor is in a msgid position, gather matching msgids, then render each as a completion item. None of this touches a catalog file — it reads the in-memory index.

```rust
// src/features/completion.rs
pub fn complete(state: &WorkspaceState, uri: &Uri, pos: Position) -> Vec<CompletionItem>;
```

### 3.1 Trigger and gating

You only want completions where a msgid belongs, not everywhere you type a quote.

**REQ-CPL-01 — Complete only inside a msgid string position.**

The server uses [F02](F02-message-extraction.md)'s call detection to find the call enclosing the cursor. The cursor must sit inside the call's msgid string literal — the first string argument. If it sits anywhere else, or in no recognized call at all, the result is empty.

**REQ-CPL-02 — Advertise the quote trigger characters.**

A string literal contains no identifier character, so editors never auto-fire completion inside one on their own. The server therefore advertises `triggerCharacters: ["\"", "'"]` in its completion capabilities, so typing either quote opens the msgid surface as the call is written.

### 3.2 Gathering candidates

You want the msgids that match what you have typed so far, with the most likely ones first.

**REQ-CPL-03 — Prefix matches first, then contains matches.**

The server reads the partial msgid typed between the opening quote and the cursor — the prefix. It walks `all_msgids` from the [catalog index](../foundations/E07-data-model.md) ([E07 REQ-IDX-04](../foundations/E07-data-model.md)) and keeps every key whose msgid contains the prefix. Keys whose msgid *starts with* the prefix sort ahead of keys that merely contain it. An empty prefix keeps every msgid.

**REQ-CPL-04 — Respect msgctxt inside a `pgettext` call.**

When the enclosing call carries a context — a `pgettext("button", …)` or its `npgettext` cousin ([E07 REQ-IDX-06](../foundations/E07-data-model.md)) — the server prefers keys whose `msgctxt` equals that context, sorting them ahead of context-free matches. A context-qualified key and a plain key are different keys ([glossary: catalog key](../glossary.md)), so both can appear; the matching-context one wins the ordering.

### 3.3 Rendering an item

Each candidate must read its msgid back into the call, and preview how it translates.

**REQ-CPL-05 — Each item carries a label, a detail, and a precise text edit.**

For each surviving key the server builds a `CompletionItem`:

- `label` is the msgid itself.
- `kind` is `TEXT` — a msgid is a plain string, not a symbol.
- `detail` previews the default locale (the first entry from `lookup`): `[de] Kasse` when translated, or `[fr] (untranslated)` when the `msgstr` is empty.
- `text_edit` replaces the partial-string range exactly — from just after the opening quote to the cursor — so picking a candidate against `"Che` yields `"Checkout"`, never a doubled `"CheCheckout"`.

```rust
// src/features/completion.rs
CompletionItem {
    label: key.msgid.clone(),
    kind: Some(CompletionItemKind::TEXT),
    detail: Some(detail_for(entries.first())),       // "[de] Kasse" | "[fr] (untranslated)"
    documentation: locale_table(&entries),           // REQ-CPL-06; None for a single locale
    text_edit: Some(CompletionTextEdit::Edit(TextEdit { range: prefix_range, new_text: key.msgid.clone() })),
    ..Default::default()
}
```

The explicit `text_edit` matters because `/`, `.`, and spaces inside a msgid break the editor's default word boundary; without it a multi-word msgid like `Bad Request` is filtered out or mis-inserted.

**REQ-CPL-06 — Show a multi-locale table when several entries exist.**

When `lookup` returns more than one entry for the key, the server renders a markdown table into `documentation`, one row per locale, so you see the whole translation picture before committing. Each row marks status — translated, untranslated, or fuzzy — alongside its `msgstr`:

```markdown
<!-- documentation value for the "Checkout" candidate -->
| Locale | Translation |
|--------|-------------|
| de     | Kasse       |
| fr     | _(untranslated)_ |
```

A key with a single entry gets no table — the `detail` line already says everything.

## 4. Examples & Use Cases

You are adding a button label in the shopfront's `app/views.py`. You type `_("Che` and the editor fires completion on the opening quote (REQ-CPL-02). The server finds the enclosing `_(…)` call, reads `Che` as the prefix, and walks the index (REQ-CPL-03).

`Checkout` is the only msgid with that prefix. The item's `label` is `Checkout`; its `detail` reads `[de] Kasse` from the German entry (REQ-CPL-05). Because `lookup` also returns the empty French entry, the `documentation` table shows both locales — `de: Kasse`, `fr: (untranslated)` (REQ-CPL-06). You accept it, the `text_edit` swaps `Che` for `Checkout`, and the literal reads `_("Checkout")`.

Later you type `pgettext("button", "Sa`. The call carries the context `button`, so the `Save` key tagged with that context sorts ahead of any plain `Save` (REQ-CPL-04).

## 5. Edge Cases & Failure Modes

- Empty prefix (cursor right after the opening quote) → every msgid in the index is offered.
- No catalogs indexed yet, or none on disk → `all_msgids` is empty → an empty result, never an error (P3).
- Cursor in a non-literal msgid position — `_(f"Hi {name}")` or `_(name)` → no msgid string to complete, so no completion ([E07](../foundations/E07-data-model.md) unresolved call).
- Cursor inside the call but outside the msgid argument (a later kwarg) → empty; only the msgid position triggers (REQ-CPL-01).
- A prefix that matches nothing → empty result; the editor shows no menu.

## 6. Cross-References

- **Depends on:** [F01-catalog-index](F01-catalog-index.md) — supplies `all_msgids` and `lookup` (REQ-CAT-05); [F02-message-extraction](F02-message-extraction.md) — supplies the enclosing-call detection and the msgid range (REQ-CPL-01).
- **Related:** [F05-hover](F05-hover.md) — renders the same multi-locale translations on a finished call; [E07-data-model](../foundations/E07-data-model.md) — `CatalogIndex`, `CatalogKey`, and `TranslationCall` (REQ-IDX-04/06).
- **Testing:** [E17 §2.5](../foundations/E17-testing.md) — this feature's row in the e2e coverage matrix.

## 7. Changelog

- **2026-06-15** — Initial draft: quote-triggered msgid completion gated on [F02](F02-message-extraction.md) call detection (REQ-CPL-01/02); prefix-then-contains candidate matching with msgctxt preference (REQ-CPL-03/04); `TEXT` items carrying a `[locale] translation` detail, a precise `textEdit`, and a multi-locale markdown table (REQ-CPL-05/06). Translated from the legacy `features/completion.rs`.
