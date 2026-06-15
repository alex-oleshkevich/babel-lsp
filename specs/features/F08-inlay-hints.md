# F08 — Inlay Hints

> **Status:** Draft
>
> **Version:** 0.1   ·   **Last updated:** 2026-06-15
>
> **Purpose:** Show the translation of each message inline, in a locale you choose, right next to the call.
>
> **Depends on:** [F01-catalog-index](F01-catalog-index.md), [F02-message-extraction](F02-message-extraction.md), [E15-app-config](../foundations/E15-app-config.md)   ·   **Related:** [F05-hover](F05-hover.md), [F12-code-lens](F12-code-lens.md)

> Requirement tag: **HINT**

---

## 1. Purpose & Scope

Inlay hints let you read a translation without leaving the source. You pick a locale, and the server renders that locale's translation right after each call.

In the shopfront, with `inlay_hint_locale = "de"`, the call `_("Checkout")` shows ` = Kasse` floating after it. You read the German next to the English, no hover, no jump.

This spec covers:

- Turning a configured `inlay_hint_locale` into one hint per resolved call.
- Where the hint sits, how it reads, and how long text is truncated.
- Refreshing hints after the catalog index changes.
- The opt-in rule and the untranslated, fuzzy, and unresolved cases.

## 2. Non-Goals / Out of Scope

- Where `inlay_hint_locale` comes from and how it's resolved — owned by [E15](../foundations/E15-app-config.md).
- Extracting the calls the hints anchor to — owned by [F02](F02-message-extraction.md).
- The full translation card on hover — owned by [F05](F05-hover.md).
- Coverage counts and translate actions on a lens — owned by [F12](F12-code-lens.md).

## 3. Detailed Specification

The feature is a pure function of the calls in a file, the requested range, the catalog index, and the chosen locale ([E07](../foundations/E07-data-model.md)).

```rust
// src/features/inlay_hints.rs
pub fn inlay_hints(state: &WorkspaceState, uri: &Uri, range: Range) -> Vec<InlayHint>;
```

**REQ-HINT-01 — Hints are off until you choose a locale.**

The feature is opt-in. When `inlay_hint_locale` is `null` — its default ([E15 REQ-CFG-04](../foundations/E15-app-config.md)) — the server returns no hints at all. A user who hasn't asked for previews sees a clean buffer.

**REQ-HINT-02 — One hint per resolved call, in the chosen locale.**

When a locale is configured, each [translation call](../glossary.md) whose msgid resolves to a translated entry in that locale gets one hint. The server looks the call's [CatalogKey](../foundations/E07-data-model.md) up in the index ([E07 REQ-IDX-04](../foundations/E07-data-model.md)) and reads the matching locale's `msgstr`. The shopfront's `_("Checkout")` resolves to the German `"Kasse"`, so the hint reads ` = Kasse`.

**REQ-HINT-03 — The hint sits after the call and reads ` = <translation>`.**

The hint is positioned at the end of the whole call expression, with a leading space, formatted as ` = <translation>`. It is a parameter-kind hint, so editors render it dimmed inline rather than as a comment. Only calls intersecting the requested `range` produce hints, so the editor pages them with the viewport.

**REQ-HINT-04 — Long translations are truncated.**

A translation longer than roughly 40 characters is cut to its first ~40 characters with a trailing `…`, so a long sentence never pushes code off-screen. The full text stays available on [hover](F05-hover.md); the hint is a glance, not the whole string.

**REQ-HINT-05 — The server refreshes hints after a relink.**

A hint depends on catalog files the user isn't editing. Adding a German `msgstr` in the open `.po` should move every hint in `views.py`, but the client caches hints per document and re-requests only on local edits. After a relink that changed catalog contents ([F01 REQ-CAT-08](F01-catalog-index.md)), the server sends `workspace/inlayHint/refresh` when the client advertises `workspace.inlayHint.refreshSupport`. Without that capability, hints stay stale until the editor next asks on its own.

## 4. Examples & Use Cases

You set the shopfront to preview German:

```toml
# pyproject.toml
[tool.babel-lsp]
inlay_hint_locale = "de"
```

You open `app/views.py`. After `_("Checkout")` the editor shows ` = Kasse`, dimmed and inline (REQ-HINT-02, REQ-HINT-03). You read the translation in place, never opening the catalog.

You scroll to `pgettext("button", "Save")`. The German `"Save"` is flagged `#, fuzzy`, so its hint reads ` = Speichern (fuzzy)` — present but marked, so you don't trust it blindly (REQ-HINT-07).

You open the French catalog and type `msgstr "Caisse"` under `"Checkout"`. The relink fires, the server refreshes, and switching `inlay_hint_locale` to `"fr"` shows ` = Caisse` on the same call (REQ-HINT-05).

## 5. Edge Cases

- **Msgid present in the locale but untranslated** (empty `msgstr`) → the hint reads ` = (untranslated)`, or shows nothing — never an empty ` = `. The shopfront's `"Checkout"` under `inlay_hint_locale = "fr"` is untranslated, so it draws this state.
- **Fuzzy translation** → the `msgstr` is shown but marked, e.g. ` = Speichern (fuzzy)`, so an unverified preview is visibly distinct from a trusted one (REQ-HINT-07).
- **Unresolved msgid** (a non-constant first argument, `msgid: None` per [E07 REQ-IDX-06](../foundations/E07-data-model.md)) → no hint; there is no msgid to look up (constitution P4).
- **Unknown msgid** (resolves in no catalog) → no hint; nothing to preview.
- **Locale configured but absent from the index** → no hints for any call; the missing locale is a config concern, not a hint.
- **Plural call** → the singular `msgstr[0]` is previewed; the full plural set lives on [hover](F05-hover.md).

**REQ-HINT-07 — Fuzzy is previewed but marked.** A `fuzzy` entry's translation is shown with a ` (fuzzy)` suffix rather than suppressed, because seeing the unverified text is more useful than blank space, as long as its status is clear.

## 6. Cross-References

- **Depends on:** [F01-catalog-index](F01-catalog-index.md) — the index the hint reads (`lookup`); [F02-message-extraction](F02-message-extraction.md) — the calls hints anchor to; [E15-app-config](../foundations/E15-app-config.md) — the `inlay_hint_locale` key.
- **Related:** [F05-hover](F05-hover.md) — the full card behind a truncated hint; [F12-code-lens](F12-code-lens.md) — coverage and translate actions; [E07-data-model](../foundations/E07-data-model.md) — `TranslationCall`, `CatalogEntry`, `CatalogKey`.
- **Testing:** [E17 §2.5](../foundations/E17-testing.md) — this feature's row in the e2e coverage matrix.

## 7. Changelog

- **2026-06-15** — Initial draft: opt-in `inlay_hint_locale` previews (REQ-HINT-01/02), the ` = <translation>` placement and ~40-char truncation (REQ-HINT-03/04), `workspace/inlayHint/refresh` after a relink (REQ-HINT-05), and the untranslated/fuzzy/unresolved cases (REQ-HINT-07). Translated from the legacy `features/inlay_hint.rs`.
