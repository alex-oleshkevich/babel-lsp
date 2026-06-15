# F09 — Symbols

> **Status:** Draft
>
> **Version:** 0.1   ·   **Last updated:** 2026-06-15
>
> **Purpose:** Catalog entries as document symbols, and any msgid searchable across the whole workspace.
>
> **Depends on:** [F01-catalog-index](F01-catalog-index.md), [E07-data-model](../foundations/E07-data-model.md)   ·   **Related:** [F05-hover](F05-hover.md), [F06-navigation](F06-navigation.md)

> Requirement tag: **SYM**

---

## 1. Purpose & Scope

This spec turns the catalog index into two editor surfaces: an outline of the catalog you're looking at, and a search box over every msgid in the workspace.

You open `de.po` and your editor's outline lists every translation in it — `Checkout`, `Save`, and the rest. You hit your workspace-symbol key, type `chec`, and the picker finds `Checkout` no matter which file you're in. Both surfaces are pure reads over the index [F01](F01-catalog-index.md) already built.

This spec covers:

- Document symbols (`textDocument/documentSymbol`) for `.po`/`.pot` files.
- Workspace symbols (`workspace/symbol`) over every msgid in the index.

## 2. Non-Goals / Out of Scope

- Symbols for Python or Jinja source — the primary language server owns those; we add only what catalogs know.
- Loading or keying catalogs — owned by [F01](F01-catalog-index.md); this spec only reads the index.
- Jumping to a symbol's definition — that's goto, owned by [F06](F06-navigation.md).

## 3. Detailed Specification

### 3.1 Document symbols

You ask for the outline of an open catalog. The server returns one symbol per entry in that file, in file order.

**REQ-SYM-01 — Each catalog entry is one document symbol.**

For a `.po`/`.pot` file, the server reads its entries via [`entries_for_file`](../foundations/E07-data-model.md) and maps each to a symbol. The symbol's `name` is the msgid; its `kind` is `STRING`; its `range` and `selection_range` anchor to the entry's `line`. The list reflects the unsaved overlay, so an entry you just typed appears before you save ([F01](F01-catalog-index.md) REQ-CAT-07).

**REQ-SYM-02 — The detail field shows status or a translation snippet.**

The `detail` summarizes the entry at a glance. A fuzzy entry reads `fuzzy`; an untranslated one reads `untranslated`; a translated one shows its `msgstr`, truncated to ~60 characters with an ellipsis. So the German `"Save"` entry shows `fuzzy`, the French `"Checkout"` shows `untranslated`, and the German `"Checkout"` shows `Kasse`.

**REQ-SYM-03 — Only catalog files produce document symbols.**

The server returns symbols only when the document's `language_id` is `po` ([E07](../foundations/E07-data-model.md)). A Python or Jinja buffer returns an empty list, so we never compete with the source language server's outline.

### 3.2 Workspace symbols

This surface is new versus the legacy server, which only had document symbols. You search across the whole catalog index, not just one file.

**REQ-SYM-04 — A query matches msgids across the whole index.**

For a `workspace/symbol` query, the server walks [`all_msgids`](../foundations/E07-data-model.md) and keeps every [CatalogKey](../foundations/E07-data-model.md) whose msgid matches. Matching is case-insensitive substring: the query `chec` matches `Checkout`, and an empty query returns every msgid. This catches partial recall — you remember "chec", not the exact casing or the full word.

**REQ-SYM-05 — Each match resolves to one location in a catalog.**

A workspace symbol needs a place to jump to. The server points each match at its `.pot` template entry when one exists, else the first defining catalog entry, using that entry's `file_path` and `line`. So searching `Checkout` lands you in `messages.pot` where the msgid is declared, the natural home of the string.

**REQ-SYM-06 — Context rides in the symbol name.**

A msgid with a `msgctxt` shows as `msgctxt|msgid` in the symbol name — `button|Save` — so the two `Save` keys read as distinct entries and a search for `button` finds the contextual one. Pickers re-filter fuzzily on the name alone, so the context must live in the name to be searchable, not in a side field the picker ignores.

### 3.3 Code map

Two pure reads over the shared state, no errors and no held state — an empty index yields an empty list.

```rust
// src/features/symbols.rs
pub fn document_symbols(state: &WorkspaceState, uri: &Uri) -> Vec<DocumentSymbol>;
pub fn workspace_symbols(state: &WorkspaceState, query: &str) -> Vec<WorkspaceSymbol>;

fn symbol_name(entry: &CatalogEntry) -> String;   // "Checkout" / "button|Save"
fn detail(entry: &CatalogEntry) -> String;        // "fuzzy" | "untranslated" | "Kasse" (REQ-SYM-02)
fn matches(key: &CatalogKey, query: &str) -> bool; // case-insensitive substring (REQ-SYM-04)
```

## 4. Examples & Use Cases

You open the shopfront's `de.po`. Your editor's outline lists `Checkout` with detail `Kasse`, and `button|Save` with detail `fuzzy` — the whole German catalog at a glance, in file order (REQ-SYM-01, REQ-SYM-02).

Later you're editing `views.py` and can't recall the exact msgid for the checkout button. You hit your workspace-symbol key and type `chec`. The picker shows `Checkout`; you select it and land in `messages.pot` where the template declares it (REQ-SYM-04, REQ-SYM-05). No catalog file was open, and no editor-specific UI was involved.

## 5. Edge Cases & Failure Modes

- The empty-msgid header entry → already dropped at load ([F01](F01-catalog-index.md) REQ-CAT-03), so it never reaches either surface.
- A huge catalog (tens of thousands of entries) → document symbols return one per entry; the count tracks the file, and a very large outline is the editor's to paginate, not ours to trim.
- A workspace query against a large index → the substring walk over `all_msgids` is linear; per P6 the index is in memory, so the scan stays fast without a separate symbol cache.
- The same msgid in many locales → workspace search returns it once, resolved to the `.pot` (or first) entry, not once per locale (REQ-SYM-05).
- A non-catalog buffer asking for document symbols → empty list (REQ-SYM-03).

## 6. Cross-References

- **Depends on:** [F01-catalog-index](F01-catalog-index.md) — supplies `entries_for_file`, `all_msgids`, and `is_in_pot`; [E07-data-model](../foundations/E07-data-model.md) — `CatalogEntry`, `CatalogKey`, and the `language_id` dispatch.
- **Related:** [F05-hover](F05-hover.md) — renders the same status/snippet on a source call; [F06-navigation](F06-navigation.md) — the goto that a symbol's location feeds.
- **Testing:** [E17 §2.5](../foundations/E17-testing.md) — this feature's row in the e2e coverage matrix.

## 7. Changelog

- **2026-06-15** — Initial draft: document symbols per catalog entry with status/snippet detail (REQ-SYM-01/02/03); new workspace-symbol search over `all_msgids` with `.pot`-first location resolution and `msgctxt` in the name (REQ-SYM-04/05/06). Translated and extended from the legacy `document_symbol.rs`.
