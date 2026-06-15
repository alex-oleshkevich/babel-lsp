# F12 — Code Lens

> **Status:** Draft
>
> **Version:** 0.1   ·   **Last updated:** 2026-06-15
>
> **Purpose:** Inline counts above your messages — how many times a msgid is used, and how many locales translate it.
>
> **Depends on:** [F01-catalog-index](F01-catalog-index.md), [F06-navigation](F06-navigation.md), [E07-data-model](../foundations/E07-data-model.md)   ·   **Related:** [F08-inlay-hints](F08-inlay-hints.md)

> Requirement tag: **LENS**

---

## 1. Purpose & Scope

A code lens is a small clickable line the editor floats above a line of code. This spec puts two of them to work for translations.

Above each catalog entry, you see how many locales translate it. Above each translation call, you see how many times that msgid is used. Click either, and you jump to the things it counts.

This spec covers:

- The catalog lens: `k of m locales translated` above each catalog entry, fuzzy marked.
- The source lens: `used N times` above each translation call.
- The click action for each, routed through [F06](F06-navigation.md) navigation.
- Refreshing lenses after a relink changes the counts.

## 2. Non-Goals / Out of Scope

- Building the index the lenses read — owned by [F01](F01-catalog-index.md).
- The goto and find-references targets a click resolves to — owned by [F06](F06-navigation.md).
- Judging an entry missing or fuzzy as a diagnostic — owned by [F03-diagnostics](F03-diagnostics.md); the lens only reports coverage, it never warns.
- Inline per-call translation previews — owned by [F08-inlay-hints](F08-inlay-hints.md).

## 3. Background & Rationale

Coverage is the question a translation team asks all day: is this string done everywhere, or only in German? The answer already lives in the [catalog index](F01-catalog-index.md) — `all_locales` and `missing_locales` ([E07 REQ-IDX-04](../foundations/E07-data-model.md)) compute it in one lookup. A lens is just that lookup, rendered above the line. It adds no new state and runs no new scan; it is a pure read, cheap by construction (per P6).

## 4. Detailed Specification

### 4.1 The catalog lens

Above each catalog entry, the lens shows how many locales translate that key out of how many exist.

**REQ-LENS-01 — A catalog entry shows its locale coverage.**

For each non-header entry in an open `.po`/`.pot` document, the server emits a lens above the entry's `msgid` line. It builds the entry's [CatalogKey](../foundations/E07-data-model.md), reads `all_locales` for the denominator and `missing_locales` for the gap, and renders `k of m locales translated` where `k = m − missing`. In the shopfront, the `"Checkout"` entry shows `1 of 2 locales translated` — German has `"Kasse"`, French is still empty.

**REQ-LENS-02 — Fuzzy translations are marked, not counted as done.**

A `#, fuzzy` entry exists but is unverified, and gettext ignores it at runtime. So a fuzzy `msgstr` does not count toward `k`. When the entry under the lens is itself fuzzy, the lens appends a marker — `1 of 2 locales translated · fuzzy` — so the translator sees the string needs a second look.

**REQ-LENS-03 — Clicking the catalog lens finds every reference.**

The catalog lens command runs find-references ([F06 REQ-NAV-04](F06-navigation.md)) on the entry's key. The result lists every other catalog entry and every source call that shares the msgid, so one click answers "where else does this live?".

### 4.2 The source lens

Above each translation call, the lens counts how many source uses share its msgid.

**REQ-LENS-04 — A translation call shows its use count.**

For each resolved [TranslationCall](../foundations/E07-data-model.md) in an open source document, the server emits a lens above the call, reading `used N times`. `N` is the number of source references — translation calls across the workspace — that resolve to the same [CatalogKey](../foundations/E07-data-model.md). Above `_("Checkout")` in `views.py`, the lens reads `used 1 time` when that is the only call using it. A call with an unresolved msgid (`msgid: None`, per P4) forms no key and gets no lens.

**REQ-LENS-05 — Clicking the source lens lists the references.**

The source lens command runs find-references ([F06](F06-navigation.md)) on the call's key, listing every source call and catalog entry that uses the msgid — the same edge the catalog lens offers, from the source side.

### 4.3 Resolve and refresh

Lenses return ranges first and fill their text lazily, then refresh when a relink moves the numbers.

**REQ-LENS-06 — Lenses resolve lazily.**

The lens pass returns ranges and a key payload immediately; the count, the coverage text, and the command fill in via `codeLens/resolve`. A large file with hundreds of entries therefore pays only for the lenses the editor actually shows. Each lens carries its [CatalogKey](../foundations/E07-data-model.md) in its `data` field so the resolve round-trip needs no second lookup of position.

**REQ-LENS-07 — The server refreshes lenses after a relink.**

A translator saves a `.po` you are not editing, and the coverage counts change in a file the editor has cached. After a relink ([E01 REQ-ARCH-04](../foundations/E01-architecture.md)) that changed counts, the server sends `workspace/codeLens/refresh` when the client advertises `workspace.codeLens.refreshSupport`. Clients without that capability re-request lenses on local edits only; the stale count clears next time the file is touched.

```rust
// src/features/codelens.rs
pub fn code_lenses(state: &WorkspaceState, uri: &Uri) -> Vec<CodeLens>;   // ranges + key, no counts
pub fn resolve(state: &WorkspaceState, lens: CodeLens) -> CodeLens;       // fills text + command

#[derive(Serialize, Deserialize)]
struct LensData { key: CatalogKey }                                       // survives the resolve round-trip
```

## 5. Examples & Use Cases

You open `locale/messages.pot` and put your cursor near `msgid "Checkout"`. Above it floats `1 of 2 locales translated`: the index reports two locales, and `missing_locales` names French (REQ-LENS-01). You open the German catalog and the `"Save"` entry reads `0 of 2 locales translated · fuzzy` — both locales lack a verified translation, and this one is flagged (REQ-LENS-02). You click the lens and find-references lists every place `"Save"` appears (REQ-LENS-03).

You switch to `app/views.py`. Above `_("Checkout")` sits `used 1 time` — the only call using that msgid (REQ-LENS-04). You add a second `_("Checkout")` in `checkout.html`; after the next pass the count becomes `used 2 times`. Meanwhile a translator saves the French `"Checkout"` translation; the relink fires `codeLens/refresh`, and the `.pot` lens flips to `2 of 2 locales translated` without you touching the file (REQ-LENS-07).

## 6. Edge Cases & Failure Modes

- A call with a non-literal msgid (`_(f"Hi {user}")`) → no key, no lens (P4).
- An entry whose key resolves in no other catalog → coverage still renders against `all_locales`; the count is honest, even at zero.
- A client with no code-lens support → no lenses appear, and that is fine. **Zed's code-lens support is limited and Helix renders none; Neovim shows lenses only after opt-in setup.** The same coverage and use-count facts are available on hover ([F05-hover](F05-hover.md)); the lens is a progressive enhancement layered over that surface, never the only way to read the number.
- A huge catalog file → lenses obey a per-file budget; past the cap the server stops emitting and lets the lazy resolve carry the visible ones, so a 5,000-entry catalog never floods the editor with lenses it will not render.
- The cursor sits in a closed file → lenses are computed only for open documents; closed files contribute to counts but show no lens of their own.

## 7. Cross-References

- **Depends on:** [F01-catalog-index](F01-catalog-index.md) — the index whose `all_locales`/`missing_locales` the coverage lens reads; [F06-navigation](F06-navigation.md) — the find-references edge both lens clicks route to; [E07-data-model](../foundations/E07-data-model.md) — `CatalogKey`, `CatalogEntry`, `TranslationCall`, and the index read API.
- **Related:** [F08-inlay-hints](F08-inlay-hints.md) — the other inline surface, per-call rather than per-count; [F05-hover](F05-hover.md) — carries the same facts where lenses do not render; [E01-architecture](../foundations/E01-architecture.md) — the relink that triggers refresh.
- **Testing:** [E17 §2.5](../foundations/E17-testing.md) — this feature's row in the e2e coverage matrix.

## 8. Changelog

- **2026-06-15** — Initial draft: the catalog coverage lens over `all_locales`/`missing_locales` with fuzzy marking (REQ-LENS-01/02), the source use-count lens (REQ-LENS-04), both clicks routed to F06 find-references (REQ-LENS-03/05), lazy resolve on a `CatalogKey` payload (REQ-LENS-06), and `codeLens/refresh` after a relink (REQ-LENS-07). Honest editor-support note (Zed limited, Helix none, Neovim opt-in) with hover as the fallback surface.
