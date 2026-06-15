# F06 — Navigation

> **Status:** Draft
>
> **Version:** 0.1   ·   **Last updated:** 2026-06-15
>
> **Purpose:** Jump between a source msgid and its catalog entries, find every use of a msgid, and click the source references inside catalogs.
>
> **Depends on:** [F01-catalog-index](F01-catalog-index.md), [F02-message-extraction](F02-message-extraction.md)   ·   **Related:** [F10-rename](F10-rename.md), [E07-data-model](../foundations/E07-data-model.md)

> Requirement tag: **NAV**

---

## 1. Purpose & Scope

You ctrl-click `_("Checkout")` and land on its `msgid "Checkout"` line in the German catalog. That clickable edge — source to translation, and back — is what this spec delivers.

It owns three LSP surfaces, all reading the [catalog index](F01-catalog-index.md) and the [translation calls](../foundations/E07-data-model.md) pass 1 extracted. None of them re-parse a catalog.

This spec covers:

- Goto definition: a source msgid → its catalog entries, `.pot` first.
- Find references: a msgid → every catalog entry and every source call that uses it.
- Document links: a `.po` file's `#:` source-reference comments become clickable.

## 2. Non-Goals / Out of Scope

- Loading and keying the catalog entries goto and references read — owned by [F01](F01-catalog-index.md).
- Extracting translation calls and their `msgid_range` from source — owned by [F02](F02-message-extraction.md).
- Renaming a msgid across these edges — owned by [F10-rename](F10-rename.md); navigation only reads, never edits.
- Judging an entry missing or fuzzy — owned by [F03-diagnostics](F03-diagnostics.md).

## 3. Detailed Specification

### 3.1 Goto definition

Goto answers "where is this msgid defined in my catalogs?". From a source call it returns every entry that defines the key, template first.

**REQ-NAV-01 — A source msgid jumps to its catalog entries.**

The cursor must sit inside a translation call's `msgid_range` ([E07 REQ-IDX-06](../foundations/E07-data-model.md)); otherwise goto returns nothing. The server builds the [CatalogKey](../foundations/E07-data-model.md) from the call's msgid and msgctxt, then collects a [Location](../foundations/E07-data-model.md) for every entry the key resolves to. Each location points at the entry's `line`, recovered by the [F01](F01-catalog-index.md) line map.

**REQ-NAV-02 — The `.pot` template comes first, then each locale.**

The result is an ordered list of locations. The `.pot` entry leads, because the template answers "is this msgid known at all?" before any locale answers "is it translated?". The `.po` entries follow, one per locale. The editor shows a picker when more than one location comes back.

```rust
// src/features/definition.rs
pub fn goto_definition(
    calls: &[TranslationCall],
    position: Position,
    index: &CatalogIndex,
) -> Option<GotoDefinitionResponse>;
```

**REQ-NAV-03 — Goto reads unsaved buffers.**

The calls passed in come from the live buffer, and the index honors the [unsaved overlay](F01-catalog-index.md) (REQ-CAT-07). So goto works on a msgid you typed seconds ago, and lands on a catalog line you are still editing — before either file is saved.

### 3.2 Find references

References answer "everywhere this msgid is used". From a msgid in *either* a source call or a catalog entry, it gathers every catalog entry and every source call that shares the key.

**REQ-NAV-04 — References aggregate catalog entries and source calls.**

The result is a flat list of [Location](../foundations/E07-data-model.md)s covering three sources:

- Every catalog entry the [CatalogKey](../foundations/E07-data-model.md) resolves to, across all locales and domains — each at its `line`.
- Every translation call in an *open* source document whose key matches — at its `msgid_range`.
- Every matching call found by scanning the workspace's on-disk source files.

```rust
// src/features/references.rs
pub fn find_references(
    calls: &[TranslationCall],
    position: Position,
    index: &CatalogIndex,
    state: &SharedState,
) -> Vec<Location>;
```

**REQ-NAV-05 — Results are deduplicated.**

A file that is both open in the editor and present on disk would otherwise yield its calls twice. The server keys each location by `(uri, range)` and keeps only the first, so an open file's buffer hit shadows its disk hit and no location repeats.

**REQ-NAV-06 — The workspace scan walks source files, skipping noise.**

The scan recurses from the workspace root, collecting `.py` files and any extension configured as Jinja ([E15](../foundations/E15-app-config.md)). It prunes the directories that never hold first-party source — `.git`, `target`, `.venv`, `venv`, `__pycache__`, `.mypy_cache`, `.pytest_cache`. Each collected file is read, extracted, and matched against the key.

### 3.3 Document links

This surface is new versus the legacy server. Inside a `.po` file, gettext records where each msgid came from in `#: path:line` comments. This spec makes those comments clickable.

**REQ-NAV-07 — `#:` reference comments become clickable links.**

For an open `.po`/`.pot` document, the server scans for reference comments and returns one `DocumentLink` per source location. The link's range covers the `path:line` text; its target is the source file, opened at that line. The user clicks a catalog's `#: app/views.py:42` and lands on line 42 of `views.py`.

```rust
// src/features/document_link.rs
pub fn document_links(state: &WorkspaceState, uri: &Uri) -> Vec<DocumentLink>;
```

**REQ-NAV-08 — Parse the `#:` reference-comment format.**

A reference comment is a line beginning `#:`, followed by one or more whitespace-separated `path:line` tokens. The path is resolved relative to the catalog's directory, then the workspace root. The server splits each token on the last colon, so a Windows-style or column-suffixed token still yields a path and a line. A token without a numeric line is skipped, not guessed at (P3/P4). The line is 1-based as gettext writes it, converted to LSP's 0-based on output.

```
#: app/views.py:42 app/templates/checkout.html:8
msgid "Checkout"
msgstr "Kasse"
```

**REQ-NAV-09 — Source-to-catalog links are optional.**

Goto definition (REQ-NAV-01) already carries source→catalog everywhere. A `documentLink` from a source call to its catalog entry is therefore a progressive enhancement: offered where it helps, never the load-bearing path. The catalog→source direction in REQ-NAV-07 is the one that matters, because no goto edge replaces it.

## 4. Examples & Use Cases

You ctrl-click `_("Checkout")` in `app/views.py`. Goto builds the key `Checkout`, and returns three locations: the `msgid "Checkout"` line in `messages.pot` first, then the German `"Kasse"` line, then the empty French entry. Your editor opens a picker; you pick German and land on `"Kasse"` (REQ-NAV-01, REQ-NAV-02).

You run find references on the same msgid. Back comes the call in `views.py`, plus the `"Checkout"` entry in all three catalogs — the `.pot`, the German `.po`, the French `.po` (REQ-NAV-04). The call in `checkout.html`, if it shares the key, joins them via the workspace scan (REQ-NAV-06).

You open the German catalog and see `#: app/views.py:42` above `"Checkout"`. You click it and jump to line 42 of `views.py` — the call this entry was extracted from (REQ-NAV-07).

## 5. Edge Cases & Failure Modes

- The cursor is not inside any call's `msgid_range` → goto and references both return nothing, no error.
- A non-literal msgid — `_(f"Hi {user}")` — has `msgid: None` ([E07 REQ-IDX-06](../foundations/E07-data-model.md)), so it forms no key; goto returns no definition (P4).
- The same msgid lives in many files → references returns every one; goto returns one location per catalog entry, deduplicated against open buffers (REQ-NAV-05).
- A `#:` reference points at a moved or deleted file → the link target still resolves to a URI, but opening it is the editor's concern; the server does not pre-check the path exists (P3).
- The workspace scan is the costliest path here: it reads and parses every source file on each references request. It is bounded by the directory pruning (REQ-NAV-06), but on a large tree it is the one navigation call that touches disk at scale, not the in-memory index.

## 6. Cross-References

- **Depends on:** [F01-catalog-index](F01-catalog-index.md) — supplies the index, the line map, and the unsaved overlay goto and references read; [F02-message-extraction](F02-message-extraction.md) — supplies the calls and their `msgid_range`.
- **Related:** [E07-data-model](../foundations/E07-data-model.md) — `CatalogKey`, `CatalogEntry.line`, `TranslationCall.msgid_range`, `Location`; [F10-rename](F10-rename.md) — the edit-side counterpart sharing the same edge set; [F03-diagnostics](F03-diagnostics.md) — judges the entries navigation lands on; [E15-app-config](../foundations/E15-app-config.md) — names the Jinja extensions the scan collects.
- **Testing:** [E17 §2.5](../foundations/E17-testing.md) — this feature's row in the e2e coverage matrix.

## 7. Changelog

- **2026-06-15** — Initial draft: goto definition with `.pot`-first ordering over unsaved buffers (REQ-NAV-01/02/03); find references aggregating catalog entries, open docs, and a pruned workspace source scan with `(uri, range)` dedup (REQ-NAV-04/05/06); document links for `#:` reference comments, new versus legacy, with the parse rule and the optional source→catalog direction (REQ-NAV-07/08/09). Translated from the legacy `features/definition.rs` and `references.rs`.
