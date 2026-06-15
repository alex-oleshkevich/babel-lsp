# F06 — Navigation

> **Status:** Draft
>
> **Version:** 0.2   ·   **Last updated:** 2026-06-15
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

The result is an ordered list of locations. The `.pot` entry leads, because the template answers "is this msgid known at all?" before any locale answers "is it translated?". The `.po` entries follow, one per locale. The editor shows a picker when more than one location comes back. The rendered list is sketched in §6.1.

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

The editor shows these as a references list or peek view; its shape is sketched in §6.1.

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

For an open `.po`/`.pot` document, the server scans for reference comments and returns one `DocumentLink` per source location. The link's range covers the `path:line` text; its target is the source file, opened at that line. The user clicks a catalog's `#: app/views.py:42` and lands on line 42 of `views.py`. The rendered link is sketched in §6.2.

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

## 6. UI Mockups

Navigation produces two editor surfaces. The first is the list the editor shows for "find all references" of a msgid — one path:line row per location. The second is a `.po` document link: the `#:` comment rendered as underlined, clickable text. Both are layout contracts the editor renders from the [Location](../foundations/E07-data-model.md)s and `DocumentLink`s this feature returns; the editor owns the actual chrome, focus, and keyboard.

### 6.1 References / locations list — "find all references" of `Checkout`

What you see when you run find references (or trigger goto's multi-result picker) on `_("Checkout")`. Each row is a `<path>:<line>` location the editor can open; the source call leads, then the catalog entries `.pot`-first.

```
  References to "Checkout" — 4 results
  ╭─ babel-lsp ──────────────────────────────────────────────────────╮
  │ <source call>                                                     │
  │   app/views.py:42            _("Checkout")                        │
  │                                                                   │
  │ <catalog entries — .pot first, then each locale>                 │
  │   locale/messages.pot:18     msgid "Checkout"                     │
  │   locale/de/LC_MESSAGES/messages.po:21   msgid "Checkout"         │
  │   locale/fr/LC_MESSAGES/messages.po:19   msgid "Checkout"         │
  ╰───────────────────────────────────────────────────────────────────╯
   <each row is clickable — opens the file at that line>
```

States: results (rows above) · single-result (goto opens the one location directly, no picker) · empty (cursor outside any `msgid_range` — no list, no error).

### 6.2 Document link — a `.po` source reference

What you see in an open German catalog: the `#:` comment above an entry. The server returns one `DocumentLink` per `path:line` token, so the editor underlines each as a clickable link.

```
  locale/de/LC_MESSAGES/messages.po
  ────────────────────────────────────────────────────────────────────
   #: app/views.py:42  app/templates/checkout.html:8
      ╰────────────╯    ╰────────────────────────╯
      <underlined link> <underlined link — second token on the same line>
   msgid "Checkout"
   msgstr "Kasse"
```

States: linked (one underline per `path:line` token) · no-line (a token without a numeric line is skipped, rendered as plain text, no underline) · no-references (an entry with no `#:` comment shows no links).

## 9. Examples & Use Cases

You ctrl-click `_("Checkout")` in `app/views.py`. Goto builds the key `Checkout`, and returns three locations: the `msgid "Checkout"` line in `messages.pot` first, then the German `"Kasse"` line, then the empty French entry. Your editor opens a picker (§6.1); you pick German and land on `"Kasse"` (REQ-NAV-01, REQ-NAV-02).

You run find references on the same msgid. Back comes the call in `views.py`, plus the `"Checkout"` entry in all three catalogs — the `.pot`, the German `.po`, the French `.po` (REQ-NAV-04). The call in `checkout.html`, if it shares the key, joins them via the workspace scan (REQ-NAV-06). The list renders as §6.1.

You open the German catalog and see `#: app/views.py:42` above `"Checkout"`. The editor underlines it (§6.2); you click it and jump to line 42 of `views.py` — the call this entry was extracted from (REQ-NAV-07).

## 10. Edge Cases & Failure Modes

- The cursor is not inside any call's `msgid_range` → goto and references both return nothing, no error.
- A non-literal msgid — `_(f"Hi {user}")` — has `msgid: None` ([E07 REQ-IDX-06](../foundations/E07-data-model.md)), so it forms no key; goto returns no definition (P4).
- The same msgid lives in many files → references returns every one; goto returns one location per catalog entry, deduplicated against open buffers (REQ-NAV-05).
- A `#:` reference points at a moved or deleted file → the link target still resolves to a URI, but opening it is the editor's concern; the server does not pre-check the path exists (P3).
- A `#:` token carries no numeric line → it is skipped, not guessed at; no link is offered for it (REQ-NAV-08, P4).
- The workspace scan is the costliest path here: it reads and parses every source file on each references request. It is bounded by the directory pruning (REQ-NAV-06), but on a large tree it is the one navigation call that touches disk at scale, not the in-memory index.

## 11. Testing

Navigation is tested by resolving the cursor to a key over the shopfront fixtures and asserting the locations, the dedup, and the parsed link ranges — against both negotiated encodings where a range is involved.

### 11.1 Scope & coverage

Target: **100% of this feature's behavior is covered.** Every `REQ-NAV-NN` below maps to at least one test; every surface state (§6) and edge case (§10) has a test. See the policy in [E17 §2](../foundations/E17-testing.md#2-coverage-policy).

### 11.2 Test plan

Each row is a behavior under test. Shared fixtures link to the [E17 registry](../foundations/E17-testing.md#5-fixtures-registry); the requirement column names what it verifies.

| Behavior / scenario | Type | Fixtures | Verifies |
|---|---|---|---|
| Goto — cursor in `_("Checkout")` returns one location per catalog entry | integration | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-NAV-01 |
| Goto ordering — the `.pot` location leads, then `de`, then `fr` | integration | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-NAV-02 |
| Goto range — each location lands on the entry's msgid line under UTF-8 and UTF-16 | integration | [non-ascii-catalog](../foundations/E17-testing.md#non-ascii-catalog) | REQ-NAV-01, REQ-NAV-02 |
| Goto on unsaved input — a just-typed msgid resolves through the overlay before save | integration | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-NAV-03 |
| References — the source call plus every catalog entry across locales come back | integration | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-NAV-04 |
| References dedup — a file open and on disk yields each `(uri, range)` once | integration | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-NAV-05 |
| Workspace scan — finds a matching call in a non-open file; prunes `.venv`/`target`/`__pycache__` | integration | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-NAV-06 |
| Document links — each `#:` `path:line` token becomes one link to the source line | integration | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-NAV-07 |
| `#:` parse — last-colon split handles a column-suffixed token; 1-based → 0-based line | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-NAV-08 |
| `#:` parse — a token with no numeric line is skipped, not guessed | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-NAV-08 |
| Link range — the underlined `path:line` range lands correctly in a multi-byte catalog | integration | [non-ascii-catalog](../foundations/E17-testing.md#non-ascii-catalog) | REQ-NAV-07, REQ-NAV-08 |
| Source→catalog link is optional — absence breaks nothing; goto still carries the edge | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-NAV-09 |

### 11.3 Fixtures

Reusable fixtures live in the [E17 registry](../foundations/E17-testing.md#5-fixtures-registry) — linked above. This feature defines no fixtures of its own; it reuses [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) for the goto/references/links behavior and the workspace scan, and [non-ascii-catalog](../foundations/E17-testing.md#non-ascii-catalog) for location and link-range correctness across the negotiated encodings.

### 11.4 Requirement coverage

Every load-bearing requirement maps to a test — this table is the proof.

| Requirement | Covered by |
|---|---|
| REQ-NAV-01 | `req_nav_01_goto_returns_a_location_per_catalog_entry` |
| REQ-NAV-02 | `req_nav_02_goto_orders_pot_first_then_locales` |
| REQ-NAV-03 | `req_nav_03_goto_reads_unsaved_buffers` |
| REQ-NAV-04 | `req_nav_04_references_aggregate_entries_and_calls` |
| REQ-NAV-05 | `req_nav_05_references_dedup_by_uri_range` |
| REQ-NAV-06 | `req_nav_06_scan_walks_sources_and_prunes_noise` |
| REQ-NAV-07 | `req_nav_07_hash_colon_comments_become_links` |
| REQ-NAV-08 | `req_nav_08_parses_last_colon_split_and_converts_line`, `req_nav_08_skips_token_without_numeric_line` |
| REQ-NAV-09 | `req_nav_09_source_to_catalog_link_is_optional` |

## 12. End-to-End Test Plan

Driving the built binary as an editor would, navigate the shopfront over the wire and assert the locations, the dedup, and the link click.

### 12.1 Coverage target

**100% of the feature's scope, end to end** — the happy path plus every reasonably possible error path (a non-literal msgid, a `#:` reference to a moved file). See the policy in [E29 §2](../foundations/E29-e2e-testing.md#2-coverage-policy).

### 12.2 Scenarios

Each scenario opens a fixture workspace, sends the real `textDocument/*` request, and asserts the response.

| # | Journey | Path | Expected outcome |
|---|---|---|---|
| E2E-01 | `textDocument/definition` on `_("Checkout")` | happy | Returns three locations — `messages.pot` first, then `de`, then `fr` — each on the entry's msgid line |
| E2E-02 | `textDocument/references` on `Checkout` | happy | Returns the `views.py` call plus the `.pot`/`de`/`fr` entries, each `(uri, range)` once |
| E2E-03 | `textDocument/documentLink` then click a `#:` link | happy | A link covers `app/views.py:42`; following it resolves to `views.py` at line 42 (0-based 41) |
| E2E-04 | `textDocument/definition` on a non-literal `_(f"Hi {user}")` | error | No definition returned — the call forms no key |
| E2E-05 | `textDocument/documentLink` for a `#:` ref to a moved file | error | A link with a resolved target is still returned; the server does not pre-check the file exists — graceful no-op |

### 12.3 Acceptance criteria & Definition of Done

The §12.2 scenarios, written Given/When/Then, are this feature's acceptance criteria:

| # | Given | When | Then |
|---|---|---|---|
| AC-01 | the clean-shopfront workspace is open | you goto-definition on `_("Checkout")` in `views.py` | three locations return — `messages.pot`, `de`, `fr` — `.pot` first, each on the msgid line |
| AC-02 | `Checkout` is used in `views.py` and all three catalogs | you find references on it | the call and the three catalog entries come back, each location exactly once |
| AC-03 | the German catalog has `#: app/views.py:42` above `"Checkout"` | you click that document link | `views.py` opens at line 42 |
| AC-04 | `views.py` has `_(f"Hi {user}")` | you goto-definition on the f-string | no definition is returned |
| AC-05 | a `#:` reference points at a file that has since moved | you request document links | a link with a resolved target is returned and following it is a graceful no-op, not an error |

**Definition of Done:** every `REQ-NAV-NN` has a passing test (§11.4), every acceptance scenario above passes, and every enabled non-functional concern (§13) is verified.

## 13. Non-Functional Requirements

### 13.1 Security & Privacy

- **Access & validation** — navigation is read-only over local files: goto and references read the in-memory index and the open buffers, and the workspace reference scan only reads files inside the workspace root. It never executes user code, opens a network connection, or shells out (P1).
- **Input & validation** — the only untrusted input is catalog/source text, parsed defensively upstream ([F01](F01-catalog-index.md)/[F02](F02-message-extraction.md)). A `#:` document-link target is resolved relative to the catalog's directory and the workspace root and never escapes it — no `..` traversal out of the tree.
- **Data sensitivity** — locations and links reference only the user's own workspace paths; no PII, secrets, or telemetry leave the process.

## 15. Open Questions & Decisions

- *(none open)*

## 16. Cross-References

- **Depends on:** [F01-catalog-index](F01-catalog-index.md) — supplies the index, the line map, and the unsaved overlay goto and references read; [F02-message-extraction](F02-message-extraction.md) — supplies the calls and their `msgid_range`.
- **Related:** [E07-data-model](../foundations/E07-data-model.md) — `CatalogKey`, `CatalogEntry.line`, `TranslationCall.msgid_range`, `Location`; [F10-rename](F10-rename.md) — the edit-side counterpart sharing the same edge set; [F03-diagnostics](F03-diagnostics.md) — judges the entries navigation lands on; [E15-app-config](../foundations/E15-app-config.md) — names the Jinja extensions the scan collects.
- **Testing:** [E17-testing](../foundations/E17-testing.md) — the coverage policy and the shared fixtures §11 reuses; [E29-e2e-testing](../foundations/E29-e2e-testing.md) — the harness and patterns §12 reuses.

## 17. Changelog

- **2026-06-15** — v0.2: restructured to the updated spec-writer template. Added §6 UI Mockups (6.1 the references/locations list, 6.2 the `#:` document link), each with a what/when intro and states; added §11 Testing (coverage, plan, fixtures, and a per-requirement table mapping REQ-NAV-01..09), §12 End-to-End Test Plan with Given/When/Then acceptance and a DoD, §13.1 Security & Privacy, and §13.2 Accessibility (content-level). Renumbered to canonical section order and cross-linked the mockups from the requirements and examples.
- **2026-06-15** — Initial draft: goto definition with `.pot`-first ordering over unsaved buffers (REQ-NAV-01/02/03); find references aggregating catalog entries, open docs, and a pruned workspace source scan with `(uri, range)` dedup (REQ-NAV-04/05/06); document links for `#:` reference comments, new versus legacy, with the parse rule and the optional source→catalog direction (REQ-NAV-07/08/09). Translated from the legacy `features/definition.rs` and `references.rs`.
</content>
</invoke>
