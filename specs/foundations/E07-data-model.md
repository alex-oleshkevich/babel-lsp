# E07 — Data Model

> **Status:** Draft
>
> **Version:** 0.1   ·   **Last updated:** 2026-06-15
>
> **Purpose:** The in-memory shapes the whole server reads: the catalog index, the per-document state, and the translation-call and catalog-entry facts.
>
> **Depends on:** [E01-architecture](E01-architecture.md), [constitution](../constitution.md)   ·   **Related:** [F01-catalog-index](../features/F01-catalog-index.md), [F02-message-extraction](../features/F02-message-extraction.md)

> Requirement tag: **IDX**

---

## 1. Purpose & Scope

This spec defines the data structures pass 2 builds and every feature reads. It owns the catalog index, the document store, and the two fact types — the translation call and the catalog entry. Feature specs reference these shapes by name rather than redefining them.

## 2. Non-Goals / Out of Scope

- *How* facts are extracted — owned by [F02](../features/F02-message-extraction.md) (source) and [F01](../features/F01-catalog-index.md) (catalogs).
- The pipeline that drives updates — owned by [E01](E01-architecture.md).

## 3. Detailed Specification

### 3.1 Workspace state

Everything the server knows lives in one struct, shared behind `Arc` and read concurrently.

**REQ-IDX-01 — One shared state, concurrent reads.**

`WorkspaceState` holds the open documents, the catalog index, the resolved config, and the workspace root. Documents live in a `DashMap` for lock-free per-entry access; the index and config sit behind `RwLock` because pass 2 swaps them wholesale. The shape, quoted from the source it describes:

```rust
// src/state.rs
pub struct WorkspaceState {
    pub documents: DashMap<Uri, DocumentState>,
    pub catalog_index: RwLock<CatalogIndex>,
    pub config: RwLock<Config>,
    pub workspace_root: RwLock<Option<PathBuf>>,
    pub generation: AtomicU64,
}
```

The `generation` counter backs the pass-1/pass-2 race guard from [E01 REQ-ARCH-04](E01-architecture.md).

### 3.2 Document state

**REQ-IDX-02 — A document carries its text, version, and language.**

Each open document stores its current text, the LSP version (for stale-edit detection), and its language id so dispatch knows whether to run the Python, Jinja, or catalog extractor:

```rust
// src/state.rs
pub struct DocumentState {
    pub text: Rope,            // ropey, for cheap incremental edits
    pub version: i32,
    pub language_id: String,   // "python" | "jinja" | "po" | …
}
```

Text is a `ropey::Rope` so an incremental `didChange` edits a slice instead of reallocating the file (per [E01 REQ-ARCH-03](E01-architecture.md)).

### 3.3 The catalog index

The index is the join table between source and translations: one lookup answers "everything known about this msgid".

**REQ-IDX-03 — The index keys entries by `(msgid, msgctxt)`.**

`CatalogIndex` maps each catalog key to every entry that defines it, across all locales and domains. A separate map holds the `.pot` template entries, since "does this msgid exist in the template?" is a distinct question from "is it translated in locale X?":

```rust
// src/catalog/index.rs
pub struct CatalogIndex {
    entries: HashMap<CatalogKey, Vec<CatalogEntry>>,  // .po entries, all locales
    pot_entries: HashMap<CatalogKey, CatalogEntry>,    // .pot template entries
    locales: BTreeSet<String>,
    domains: BTreeSet<String>,
}

pub struct CatalogKey { pub msgid: String, pub msgctxt: Option<String> }
```

**REQ-IDX-04 — The index answers the queries features need.**

The index exposes a read API the features lean on — none of them walk the maps directly:

```rust
// src/catalog/index.rs
impl CatalogIndex {
    pub fn lookup(&self, key: &CatalogKey) -> &[CatalogEntry];
    pub fn all_msgids(&self) -> impl Iterator<Item = &CatalogKey>;
    pub fn all_locales(&self) -> &BTreeSet<String>;
    pub fn all_domains(&self) -> &BTreeSet<String>;
    pub fn missing_locales(&self, key: &CatalogKey) -> Vec<String>;  // locales with no/empty msgstr
    pub fn entries_for_file(&self, path: &Path) -> Vec<&CatalogEntry>;
    pub fn is_in_pot(&self, key: &CatalogKey) -> bool;
}
```

`lookup` powers hover and completion; `missing_locales` powers the missing-translation diagnostic and the coverage lens; `is_in_pot` separates "unknown msgid" from "known but untranslated".

### 3.4 The catalog entry

**REQ-IDX-05 — An entry records its translation, its location, and its status.**

A `CatalogEntry` is one parsed record, carrying enough to render hover, locate the definition, and judge its status:

```rust
// src/catalog/index.rs
pub struct CatalogEntry {
    pub locale: String,            // "de", "fr", or "" for the .pot template
    pub domain: String,            // "messages", "admin", …
    pub msgid: String,
    pub msgctxt: Option<String>,
    pub msgid_plural: Option<String>,
    pub msgstr: Vec<String>,       // one slot, or many for plurals
    pub flags: EntryFlags,         // { fuzzy: bool, obsolete: bool }
    pub file_path: PathBuf,
    pub line: u32,                 // 1-based line of the msgid
}
```

`locale` and `domain` derive from the file's path — `locale/de/LC_MESSAGES/messages.po` yields `("de", "messages")` (the rule lives in [F01](../features/F01-catalog-index.md)).

### 3.5 The translation call

**REQ-IDX-06 — A call records its function, message, and ranges.**

A `TranslationCall` is the source-side fact: one recognized gettext-variant call, with the ranges features anchor to. The `msgid_range` is the string literal alone, so hover, goto, and rename target the msgid, not the whole call:

```rust
// src/extract/types.rs
pub struct TranslationCall {
    pub func: TranslationFunc,
    pub msgid: Option<String>,        // None when the first arg isn't a literal (unresolved)
    pub msgid_plural: Option<String>,
    pub msgctxt: Option<String>,
    pub domain: Option<String>,
    pub range: Range,                 // the whole call expression
    pub msgid_range: Option<Range>,   // the msgid string literal
}

pub enum TranslationFunc {
    Gettext, NGettext, PGettext, NPGettext,
    DGettext, DNGettext, DPGettext, DNPGettext,
}
```

`func` determines the argument layout — which positions hold the plural, context, and domain — so the extractor reads the right strings ([F02](../features/F02-message-extraction.md)). A `None` msgid marks an unresolved call, kept for reporting and skipped by lookups (constitution P4).

### 3.6 The unsaved overlay

**REQ-IDX-07 — Open catalog buffers shadow the disk.**

When pass 2 builds the index, any `.po`/`.pot` open in the editor contributes its *buffer* text, not its on-disk text. The index therefore reflects what the user sees while typing in a catalog, and source-side diagnostics update live as a translation is added. The overlay is keyed by URI against the `documents` map.

## 4. Visualizations

The index is a join: one source msgid fans out to its entries across locales.

```mermaid
flowchart TB
    classDef src fill:#CCE5FF,stroke:#4A90D9,color:#004085
    classDef key fill:#FFF3CD,stroke:#FFC107,color:#333
    classDef entry fill:#D4EDDA,stroke:#28A745,color:#155724

    call["_(\"Checkout\")\nin views.py"]:::src
    key["CatalogKey\nmsgid=Checkout"]:::key
    de["de: 'Kasse'"]:::entry
    fr["fr: (missing)"]:::entry
    pot["pot: template"]:::entry

    call -->|"resolve msgid"| key
    key -->|"lookup"| de
    key -->|"lookup"| fr
    key -->|"is_in_pot"| pot

    linkStyle 0 stroke:#4A90D9,stroke-width:2px
    linkStyle 1 stroke:#28A745,stroke-width:2px
    linkStyle 2 stroke:#28A745,stroke-width:2px
    linkStyle 3 stroke:#FFC107,stroke-width:2px
```

## 5. Edge Cases & Failure Modes

- Same msgid, different msgctxt → two distinct keys, never merged.
- A plural entry whose `msgstr` count disagrees with the locale's `nplurals` → stored as-is; the mismatch is a diagnostic ([F03](../features/F03-diagnostics.md)), not a parse failure.
- A call with a non-literal msgid → `msgid: None`; it appears in no lookup but can still carry a diagnostic ([F03](../features/F03-diagnostics.md) `msg/non-constant-id`).
- The same `(msgid, msgctxt)` appearing twice in one file → both entries are kept; the duplicate is a diagnostic.

## 6. Cross-References

- **Depends on:** [E01-architecture](E01-architecture.md) — the pipeline that fills these structures; [constitution](../constitution.md) — P4 unresolved handling.
- **Related:** [F01-catalog-index](../features/F01-catalog-index.md) — how entries are loaded and keyed; [F02-message-extraction](../features/F02-message-extraction.md) — how calls are extracted.

## 7. Changelog

- **2026-06-15** — Initial draft: `WorkspaceState`, `DocumentState` (ropey-backed), `CatalogIndex` with the `(msgid, msgctxt)` key and the `.pot` side-map, `CatalogEntry`, `TranslationCall`/`TranslationFunc`, and the unsaved overlay rule.
