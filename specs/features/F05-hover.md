# F05 — Hover

> **Status:** Draft
>
> **Version:** 0.3   ·   **Last updated:** 2026-06-15
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

The returned hover's range is the `msgid_range` — the string literal alone ([E07 REQ-IDX-06](../foundations/E07-data-model.md)). The editor underlines just the msgid, so the highlight matches what the card is about. The rendered shape is sketched in §6.

## 6. UI Mockups

Hover produces one editor surface: a popover card anchored under the msgid the cursor sits on. The card always names the message, then either tables its translations or says no catalog knows it. The mockups below are the layout contract for that card in its three shapes — a plain key, a context key, and the empty case. The markdown source these render from lives in §9.

### 6.1 Plain hover card — `_("Checkout")`

What you see when you hover a plain `gettext` call whose msgid is in the catalogs. The card names the msgid, then tables one row per locale.

```
  _("Checkout")
    ╰────────╯
  ╭─ babel-lsp ──────────────────────────────────╮
  │ msgid  Checkout                              │
  │                                              │
  │ Locale   Domain     Status      Translation  │
  │ de       messages   ok          Kasse        │
  │ fr       messages   missing      —           │
  ╰──────────────────────────────────────────────╯
```

States: has-translations (rows above) · fuzzy (the row's status reads `fuzzy` — see §6.2) · no-translations-found (no table — see §6.3).

### 6.2 Context hover card — `pgettext("button", "Save")`

What you see when you hover a `pgettext` call. The key carries an `msgctxt`, so the card adds a `context` line above the table; here German marked the entry `fuzzy`.

```
  pgettext("button", "Save")
                     ╰────╯
  ╭─ babel-lsp ──────────────────────────────────╮
  │ msgid    Save                                │
  │ context  button                              │
  │                                              │
  │ Locale   Domain     Status      Translation  │
  │ de       messages   fuzzy       Speichern    │
  │ fr       messages   missing      —           │
  ╰──────────────────────────────────────────────╯
```

States: has-translations · fuzzy (German row, above — the translation exists but is unverified, and gettext ignores it at runtime) · no-translations-found. A plural call (`ngettext`) adds a `plural` line below `context` the same way.

### 6.3 No-translations-found card — the typo case

What you see when the msgid resolves to no catalog key — the `_("Chekout")` typo. The card still names the message, then says no catalog knows it instead of showing an empty grid.

```
  _("Chekout")
    ╰───────╯
  ╭─ babel-lsp ──────────────────────────────────╮
  │ msgid  Chekout                               │
  │                                              │
  │ No translations found.                       │
  ╰──────────────────────────────────────────────╯
```

States: no-translations-found (the only state — there are no rows to show).

## 9. Examples & Use Cases

You hover `_("Checkout")` in the shopfront's `views.py`. The provider finds the call, resolves the key `(Checkout, None)`, and calls `lookup`. Two entries come back — German `Kasse`, French empty — and the card renders from this markdown (its rendered shape is §6.1):

```markdown
**msgid** `Checkout`

| Locale | Domain | Translation | Status |
|--------|--------|-------------|--------|
| de | messages | Kasse | ok |
| fr | messages | — | missing |
```

German is translated; French still needs work — and you saw it without leaving the source line.

Now you hover `pgettext("button", "Save")`. The key carries a context, and German marked this one fuzzy, so the markdown adds a context line above the table (rendered shape: §6.2):

```markdown
**msgid** `Save`

**context** `button`

| Locale | Domain | Translation | Status |
|--------|--------|-------------|--------|
| de | messages | Speichern | fuzzy |
| fr | messages | — | missing |
```

The fuzzy German row warns you the translation exists but is unverified — gettext ignores it at runtime, and now so do you.

Hover `ngettext("%(num)d item", "%(num)d items", n)` and the card adds the plural line:

```markdown
**msgid** `%(num)d item`

**plural** `%(num)d items`
```

## 10. Edge Cases & Failure Modes

- Cursor on a non-literal msgid — `_(f"Hello {user}")` — → the call is [unresolved](../glossary.md), its `msgid` is `None`, so there is no key and no hover (P4).
- Cursor in the source call but outside the `msgid_range` (on the function name or a later arg) → null, not a card.
- A msgid defined in several domains — `messages` and `admin` → each domain gets its own rows, so one locale can appear twice, distinct per domain.
- A fuzzy entry → shown as `fuzzy`, never silently rendered as `ok`.
- An [obsolete](../glossary.md) entry → not shown; it no longer describes a live msgid.
- Hovering the msgid line inside a `.po` buffer → the same card, so you see sibling locales while editing one catalog.

## 11. Testing

Hover is tested by resolving the cursor to a key over the shopfront fixtures and asserting the rendered card and its anchor range, against both negotiated encodings.

### 11.1 Scope & coverage

Target: **100% of this feature's behavior is covered.** Every `REQ-HOV-NN` below maps to at least one test; every card state (§6) and edge case (§10) has a test. See the policy in [E17 §2](../foundations/E17-testing.md#2-coverage-policy).

### 11.2 Test plan

Each row is a behavior under test. Shared fixtures link to the [E17 registry](../foundations/E17-testing.md#5-fixtures-registry); the requirement column names what it verifies.

| Behavior / scenario | Type | Fixtures | Verifies |
|---|---|---|---|
| Dispatch on a source call — cursor in `_("Checkout")` resolves the key | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HOV-01 |
| Dispatch on a catalog entry — cursor on a `.po` msgid line resolves the same key | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HOV-01 |
| Header — id only for `gettext`; id + context for `pgettext`; id + plural for `ngettext` | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HOV-02 |
| Per-locale table — one row per entry, sorted by locale then domain | integration | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HOV-02 |
| Status — `ok` for non-empty, `fuzzy` for the flagged entry, `missing` rendering `—` for empty | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HOV-03 |
| No-translations case — the typo'd msgid renders "No translations found", not an empty grid | integration | [unknown-msgid](../foundations/E17-testing.md#unknown-msgid) | REQ-HOV-04 |
| msgid-range anchor — the hover range is the string literal under both UTF-8 and UTF-16 | integration | [non-ascii-catalog](../foundations/E17-testing.md#non-ascii-catalog) | REQ-HOV-05 |

### 11.3 Fixtures

Reusable fixtures live in the [E17 registry](../foundations/E17-testing.md#5-fixtures-registry) — linked above. This feature defines no fixtures of its own; it reuses [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) for the has-translations/fuzzy/missing rows, [unknown-msgid](../foundations/E17-testing.md#unknown-msgid) for the no-translations case, and [non-ascii-catalog](../foundations/E17-testing.md#non-ascii-catalog) for range correctness across encodings.

### 11.4 Requirement coverage

Every load-bearing requirement maps to a test — this table is the proof.

| Requirement | Covered by |
|---|---|
| REQ-HOV-01 | `req_hov_01_dispatches_on_source_call`, `req_hov_01_dispatches_on_catalog_entry` |
| REQ-HOV-02 | `req_hov_02_renders_id_context_plural_header`, `req_hov_02_renders_per_locale_table` |
| REQ-HOV-03 | `req_hov_03_status_is_ok_fuzzy_or_missing` |
| REQ-HOV-04 | `req_hov_04_no_entries_says_no_translations_found` |
| REQ-HOV-05 | `req_hov_05_anchors_to_msgid_range_both_encodings` |

## 12. End-to-End Test Plan

Driving the built binary as an editor would, hover over the shopfront and assert the card content and its range over the wire.

### 12.1 Coverage target

**100% of the feature's scope, end to end** — the happy path plus the reasonably possible error paths (a typo'd msgid, a non-literal call, the encoding edge). See the policy in [E29 §2](../foundations/E29-e2e-testing.md#2-coverage-policy).

### 12.2 Scenarios

Each scenario opens a fixture workspace, sends a `textDocument/hover`, and asserts the response.

| # | Journey | Path | Expected outcome |
|---|---|---|---|
| E2E-01 | Hover `_("Checkout")` | happy | Card names `Checkout`; table shows `de` = `ok` (Kasse), `fr` = `missing` (`—`) |
| E2E-02 | Hover a fuzzy entry (`pgettext("button", "Save")`) | happy | Card adds the `context` line; the German row's status reads `fuzzy` |
| E2E-03 | Hover the typo'd `_("Chekout")` | error | Card names `Chekout`, then "No translations found" — no table |
| E2E-04 | Hover a non-literal msgid (`_(f"Hello {user}")`) | error | No hover returned — the call is unresolved |
| E2E-05 | Hover in a non-ASCII catalog | error | The hover range lands on the msgid literal under both UTF-8 and UTF-16 |

### 12.3 Acceptance criteria & Definition of Done

The §12.2 scenarios, written Given/When/Then, are this feature's acceptance criteria:

| # | Given | When | Then |
|---|---|---|---|
| AC-01 | the clean-shopfront workspace is open | you hover `_("Checkout")` in `views.py` | a card names `Checkout` and tables `de` = `ok`, `fr` = `missing` |
| AC-02 | German's `Save` is `#, fuzzy` | you hover `pgettext("button", "Save")` | the card carries a `context` line and marks the German row `fuzzy` |
| AC-03 | `views.py` has a typo'd `_("Chekout")` | you hover it | the card says "No translations found" rather than an empty table |
| AC-04 | `views.py` has `_(f"Hello {user}")` | you hover the f-string | no hover is returned |
| AC-05 | a non-ASCII catalog is loaded | you hover its msgid | the hover range covers exactly the literal under either negotiated encoding |

**Definition of Done:** every `REQ-HOV-NN` has a passing test (§11.4), every acceptance scenario above passes, and every enabled non-functional concern (§13) is verified.

## 13. Non-Functional Requirements

### 13.1 Security & Privacy

- **Access & validation** — hover is a read-only render of local catalog data already loaded into the index; it never executes user code, opens a network connection, or shells out (P1).
- **Data sensitivity** — the card shows only msgids and translations from the user's own workspace; no PII, secrets, or telemetry leave the process.
- **Baseline** — the only untrusted input is catalog/source text, parsed defensively upstream ([F01](F01-catalog-index.md)/[F02](F02-message-extraction.md)); hover renders the resulting facts and adds no new trust boundary.

## 16. Cross-References

- **Depends on:** [F01-catalog-index](F01-catalog-index.md) — supplies the index hover reads via `lookup`; [F02-message-extraction](F02-message-extraction.md) — supplies the `TranslationCall` and its `msgid_range`.
- **Related:** [E07-data-model](../foundations/E07-data-model.md) — `CatalogEntry`, `CatalogKey`, and the `lookup` API hover calls; [F04-completion](F04-completion.md) — the other reader of the index, sharing the locale/status vocabulary.
- **Testing:** [E17-testing](../foundations/E17-testing.md) — the coverage policy and the shared fixtures §11 reuses; [E29-e2e-testing](../foundations/E29-e2e-testing.md) — the harness and patterns §12 reuses.

## 17. Changelog

- **2026-06-15** — v0.3: restructured to the updated spec-writer template. Moved the rendered hover-card ASCII mockups into a new §6 UI Mockups (6.1 plain, 6.2 context, 6.3 no-translations-found), each with a what/when intro and states; kept the markdown-source walk-through in §9 Examples and cross-linked the rendered shapes. Added §11 Testing (coverage, plan, fixtures, and a per-requirement coverage table mapping REQ-HOV-01..05), §12 End-to-End Test Plan with Given/When/Then acceptance and a DoD, §13.1 Security & Privacy, and §13.2 Accessibility (content-level). Renumbered to canonical section order.
- **2026-06-15** — v0.2: added ASCII mockups showing how the hover card renders in the editor for the plain and `pgettext` (context) cases, anchored under the msgid (per the constitution §6 editor-surface-mockup allowance).
- **2026-06-15** — Initial draft: one provider dispatching on source calls and catalog entries (REQ-HOV-01); the id/context/plural header plus the per-locale translation table (REQ-HOV-02/03); the "no translations found" case (REQ-HOV-04); the msgid-range anchor (REQ-HOV-05). Translated from the legacy `features/hover.rs`.
