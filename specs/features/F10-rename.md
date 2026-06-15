# F10 — Rename

> **Status:** Draft
>
> **Version:** 0.1   ·   **Last updated:** 2026-06-15
>
> **Purpose:** Rename a msgid once and update every catalog entry and call site across the workspace in a single edit.
>
> **Depends on:** [F01-catalog-index](F01-catalog-index.md), [F06-navigation](F06-navigation.md), [E07-data-model](../foundations/E07-data-model.md)   ·   **Related:** [F03-diagnostics](F03-diagnostics.md)

> Requirement tag: **RNM**

---

## 1. Purpose & Scope

You put the cursor on `"Checkout"`, ask your editor to rename it to `"Checkout page"`, and every place that names that msgid changes at once — the source call, the template, and all three catalogs. That single coordinated edit is what this spec delivers.

This spec covers:

- `textDocument/prepareRename`: confirm the cursor sits on a renameable msgid and return its range.
- `textDocument/rename`: build one `WorkspaceEdit` that rewrites the msgid everywhere it appears.
- The gettext nuance of re-keying a message, and the collision case.

## 2. Non-Goals / Out of Scope

- Finding the entries and calls a rename edits — the edge set is owned by [F06](F06-navigation.md); rename reuses its reference-finding and only adds the edits.
- Loading and keying catalog entries — owned by [F01](F01-catalog-index.md).
- Renaming a msgctxt, a locale, or a domain — only the msgid string is renameable here.
- Running `pybabel update` or migrating translations — the catalog stays the translator's tool (constitution P5).

## 3. Detailed Specification

Rename works on a [CatalogKey](../foundations/E07-data-model.md). The cursor identifies one key; the edit rewrites that key's msgid in every catalog entry and every source call that shares it. The msgctxt never changes, so the key's context half is preserved.

### 3.1 Prepare rename

`prepareRename` is the gate. It answers "is there a msgid here I can rename?" and hands back the exact range the editor should highlight.

**REQ-RNM-01 — Prepare returns the msgid range under the cursor.**

The server accepts the position in one of two places. In a source document, the cursor must sit inside a translation call's `msgid_range` ([E07 REQ-IDX-06](../foundations/E07-data-model.md)); the returned range is that literal alone. In a catalog, the cursor must sit on a `msgid` line of a real entry; the returned range covers the quoted msgid text. Anywhere else, prepare returns nothing and the editor refuses the rename.

```rust
// src/features/rename.rs
pub fn prepare_rename(
    state: &WorkspaceState,
    uri: &Uri,
    position: Position,
) -> Option<PrepareRenameResponse>;
```

**REQ-RNM-02 — Non-renameable positions are rejected.**

A non-literal msgid — `_(f"Hi {user}")` — has `msgid: None` ([E07 REQ-IDX-06](../foundations/E07-data-model.md)), so it forms no key and prepare rejects it (constitution P4). A catalog header entry (empty msgid) is rejected too; renaming the header is meaningless. Rejection is silent — `None`, never an error.

### 3.2 Rename

Rename resolves the key under the cursor, then collects every edit that rewrites it, across catalogs and source alike.

**REQ-RNM-03 — Rename builds one WorkspaceEdit covering every catalog entry.**

From the resolved [CatalogKey](../foundations/E07-data-model.md), the server looks up every entry the key defines across all locales and the `.pot` template ([E07 REQ-IDX-04](../foundations/E07-data-model.md)). For each, it emits a `TextEdit` replacing the `msgid` line's quoted text with the new name, escaped for the `.po` format and preserving the obsolete (`#~`) prefix where present. The msgstr and msgctxt lines are untouched.

**REQ-RNM-04 — Rename also rewrites every source call site.**

This is the improvement over the legacy server, which renamed catalogs and open source buffers but left on-disk source unsearched — a rename could miss a call in a file you hadn't opened. Here the source side is workspace-wide. The server rewrites the `msgid_range` of every matching call in open documents *and* in the on-disk source scan, reusing [F06](F06-navigation.md)'s reference-finding (REQ-NAV-04/06) and its pruning rules. Results are deduplicated by `(uri, range)` so an open file's buffer hit shadows its disk hit (REQ-NAV-05).

```rust
// src/features/rename.rs
pub fn rename(
    state: &WorkspaceState,
    uri: &Uri,
    position: Position,
    new_name: &str,
) -> Option<WorkspaceEdit>;
```

**REQ-RNM-05 — The edit respects the unsaved overlay.**

Every catalog entry rewritten reads from the buffer when one is open, not the disk copy ([E07 REQ-IDX-07](../foundations/E07-data-model.md), [F01](F01-catalog-index.md) REQ-CAT-07). So a rename lands correctly on lines you are still editing, and the `WorkspaceEdit` the editor applies is consistent with what you see on screen.

### 3.3 The gettext nuance

Renaming a msgid is not a cosmetic edit — it re-keys the message. You should understand what that does to your catalogs before you reach for it.

**REQ-RNM-06 — Rename re-keys in place and keeps catalogs in sync.**

The edit changes the `msgid` line text in every catalog and the literal in every source call together, so source and catalog stay linked under the new key. But this diverges from a `pybabel update`. Normally a changed msgid becomes a fresh, untranslated key, and the old translation drops to obsolete or fuzzy. This rename instead edits the entries in place, so the German `"Kasse"` stays attached to the new msgid rather than being orphaned. That is usually what you want from an editor refactor — but it is your edit, not gettext's migration, and the spec is explicit about the difference (constitution P5).

### 3.4 Collisions

Renaming onto a msgid that already exists would merge two distinct messages — the server will not do that silently.

**REQ-RNM-07 — A collision aborts the rename with a message.**

Before building the edit, the server checks whether the target — `(new_name, msgctxt)` — already resolves to an entry in the index. If it does, rename aborts and returns an error the editor surfaces: the two messages would merge, and the server cannot know that is intended. Renaming `"Checkout"` to an existing `"Cart"` is refused; pick a free name, or merge the catalogs by hand first. Abort, never merge.

## 4. Examples & Use Cases

You decide `"Checkout"` should read `"Checkout page"`. You put the cursor on the literal in `app/views.py` and invoke rename.

Prepare confirms the cursor is inside the call's `msgid_range` and highlights `Checkout` (REQ-RNM-01). You type the new name. The server resolves the key `Checkout`, checks that `"Checkout page"` is free (REQ-RNM-07), and builds one `WorkspaceEdit`:

```rust
// the changes map, one entry per touched file
views.py            // _("Checkout")              → _("Checkout page")
templates/checkout.html  // the matching call, found by the scan
messages.pot        // msgid "Checkout"           → msgid "Checkout page"
de/…/messages.po    // msgid "Checkout"  (Kasse kept)   → msgid "Checkout page"
fr/…/messages.po    // msgid "Checkout"  (still empty)  → msgid "Checkout page"
```

You accept. All five files change at once. The German `"Kasse"` rides along, still attached to the renamed key (REQ-RNM-06). Goto and hover on the new msgid resolve exactly as before, because every edge moved together.

## 5. Edge Cases & Failure Modes

- Cursor not on a msgid → prepare returns `None`; the editor refuses the rename, no error.
- Non-literal msgid (`_(f"Hi {user}")`) → no key, prepare rejects it (REQ-RNM-02, P4).
- Cursor on a catalog header entry → rejected; the header is not a renameable msgid.
- Target msgid already exists → rename aborts with a merge-warning message (REQ-RNM-07).
- Renaming inside a `fuzzy` entry → the msgid still changes; the `#, fuzzy` flag is left as-is, because rename judges no translation. Whether the now-renamed entry is still fuzzy is the translator's call, surfaced by [F03](F03-diagnostics.md), not rewritten here.
- An obsolete (`#~`) entry sharing the key → its msgid line is rewritten too, with the `#~` prefix preserved.
- The on-disk scan is the costly path, exactly as in [F06](F06-navigation.md) (REQ-NAV-06): rename reads and parses every source file once to find call sites. Bounded by the same directory pruning.

## 6. Cross-References

- **Depends on:** [F01-catalog-index](F01-catalog-index.md) — supplies the index, the line map, and the unsaved overlay the edits read; [F06-navigation](F06-navigation.md) — supplies the reference-finding (open docs + workspace scan + dedup) rename reuses; [E07-data-model](../foundations/E07-data-model.md) — `CatalogKey`, `CatalogEntry.line`, `TranslationCall.msgid_range`.
- **Related:** [F03-diagnostics](F03-diagnostics.md) — judges fuzzy/missing status after a rename; rename never sets those flags itself.
- **Testing:** [E17 §2.5](../foundations/E17-testing.md) — this feature's row in the e2e coverage matrix.

## 7. Changelog

- **2026-06-15** — Initial draft: `prepareRename` gate for source and catalog msgids with non-literal/header rejection (REQ-RNM-01/02); workspace-wide `rename` over all catalogs and all source call sites, reusing F06 reference-finding and the unsaved overlay (REQ-RNM-03/04/05); the re-key-in-place nuance versus `pybabel update` (REQ-RNM-06); collision abort (REQ-RNM-07). Translated from the legacy `features/rename.rs`, extending its source-side reach from open buffers to the full workspace scan.
