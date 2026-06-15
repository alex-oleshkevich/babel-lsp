# F08 — Inlay Hints

> **Status:** Draft
>
> **Version:** 0.2   ·   **Last updated:** 2026-06-15
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

## 6. UI Mockups

A hint is a single run of dimmed text the editor floats at the end of a translation call — there is no panel or popover, just the ` = <translation>` appended inline. These sketches show what a line of `views.py` looks like once the server has answered, with `inlay_hint_locale = "de"`.

### 6.1 Inline hint — `_("Checkout")` previewed in German

This is what you see when a call resolves to a translated entry in the chosen locale. The `‹…›` marks the dimmed hint the editor renders after the call; everything to its left is your real source.

```
  return render("checkout.html", title=_("Checkout")‹ = Kasse›)
```

States:

- **hint shown** — ` = Kasse`, the resolved `msgstr`, dimmed inline (REQ-HINT-02).
- **untranslated** — the locale has the msgid but its `msgstr` is empty, so the hint reads ` = (untranslated)` rather than a bare ` = ` (REQ-HINT-06).
- **fuzzy** — the entry is flagged `#, fuzzy`, so the hint adds a marker word: ` = Speichern (fuzzy)` (REQ-HINT-07).
- **disabled** — no `inlay_hint_locale` is set, so no hint is appended at all; the line is plain source (REQ-HINT-01).

```
hint shown:    …, title=_("Checkout")‹ = Kasse›)
untranslated:  …, title=_("Checkout")‹ = (untranslated)›)
fuzzy:         …pgettext("button", "Save")‹ = Speichern (fuzzy)›)
disabled:      …, title=_("Checkout"))
```

## 7. Visualizations

How one request turns into the hints the editor draws — the opt-in gate, the per-call lookup, and the untranslated/fuzzy/truncation branches.

```mermaid
flowchart TD
    Req[inlay_hints request for a range] --> Gate{inlay_hint_locale set?}
    Gate -- no --> None[Return no hints]
    Gate -- yes --> Calls[Calls intersecting the range]
    Calls --> Lookup{msgid resolves<br/>in this locale?}
    Lookup -- no --> Skip[No hint for this call]
    Lookup -- yes --> Empty{msgstr empty?}
    Empty -- yes --> Untrans[= (untranslated)]
    Empty -- no --> Fuzzy{fuzzy?}
    Fuzzy -- yes --> Mark[= text (fuzzy)]
    Fuzzy -- no --> Plain[= text]
    Untrans --> Trunc[Truncate past ~40 chars]
    Mark --> Trunc
    Plain --> Trunc
    Trunc --> Emit[Emit hint after the call]

    classDef stop fill:#fde8e8,stroke:#c0392b,color:#7b241c;
    classDef emit fill:#e8f6ef,stroke:#27ae60,color:#196f3d;
    class None,Skip stop;
    class Emit emit;
```

## 8. Data Shapes

A resolved hint crosses the LSP boundary as a single `InlayHint`. The `label` carries the ` = <translation>` text, `paddingLeft` keeps it off the call, and `kind` 2 (parameter) is what editors dim:

```json
{
  "position": { "line": 12, "character": 41 },
  "label": " = Kasse",
  "kind": 2,
  "paddingLeft": false
}
```

## 9. Examples & Use Cases

You set the shopfront to preview German:

```toml
# pyproject.toml
[tool.babel-lsp]
inlay_hint_locale = "de"
```

You open `app/views.py`. After `_("Checkout")` the editor shows ` = Kasse`, dimmed and inline (REQ-HINT-02, REQ-HINT-03). You read the translation in place, never opening the catalog.

You scroll to `pgettext("button", "Save")`. The German `"Save"` is flagged `#, fuzzy`, so its hint reads ` = Speichern (fuzzy)` — present but marked, so you don't trust it blindly (REQ-HINT-07).

You open the French catalog and type `msgstr "Caisse"` under `"Checkout"`. The relink fires, the server refreshes, and switching `inlay_hint_locale` to `"fr"` shows ` = Caisse` on the same call (REQ-HINT-05).

## 10. Edge Cases & Failure Modes

- **Msgid present in the locale but untranslated** (empty `msgstr`) → the hint reads ` = (untranslated)`, never an empty ` = ` (REQ-HINT-06). The shopfront's `"Checkout"` under `inlay_hint_locale = "fr"` is untranslated, so it draws this state.
- **Fuzzy translation** → the `msgstr` is shown but marked, e.g. ` = Speichern (fuzzy)`, so an unverified preview is visibly distinct from a trusted one (REQ-HINT-07).
- **Unresolved msgid** (a non-constant first argument, `msgid: None` per [E07 REQ-IDX-06](../foundations/E07-data-model.md)) → no hint; there is no msgid to look up (constitution P4).
- **Unknown msgid** (resolves in no catalog) → no hint; nothing to preview.
- **Locale configured but absent from the index** → no hints for any call; the missing locale is a config concern, not a hint.
- **Plural call** → the singular `msgstr[0]` is previewed; the full plural set lives on [hover](F05-hover.md).

**REQ-HINT-06 — Untranslated is named, never blank.** A resolved entry whose `msgstr` is empty draws ` = (untranslated)` rather than a bare ` = `, so the empty state is a word the reader can recognize and not mistaken for a render bug.

**REQ-HINT-07 — Fuzzy is previewed but marked.** A `fuzzy` entry's translation is shown with a ` (fuzzy)` suffix rather than suppressed, because seeing the unverified text is more useful than blank space, as long as its status is clear.

## 11. Testing

A hint is a pure function of the calls, the range, the index, and the locale, so most of it is unit-tested against the shopfront, with integration tests covering the relink-refresh path.

### 11.1 Scope & coverage

Target: **100% of this feature's behavior is covered.** Every `REQ-HINT-NN` below maps to at least one test; every hint state (§6) and edge case (§10) has a test. See the policy in [E17 §2](../foundations/E17-testing.md#2-coverage-policy).

### 11.2 Test plan

Each row is a behavior under test. Shared fixtures link to the [E17 registry](../foundations/E17-testing.md#5-fixtures-registry); the requirement column names what it verifies.

| Behavior / scenario | Type | Fixtures | Verifies |
|---|---|---|---|
| No locale set → the server returns zero hints | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HINT-01 |
| Locale set → `_("Checkout")` gets one hint reading ` = Kasse` | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HINT-02 |
| Placement — the hint sits after the whole call, formatted ` = <translation>` | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HINT-03 |
| Only calls intersecting the requested `range` produce hints | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HINT-03 |
| A translation past ~40 chars is cut to ~40 with a trailing `…` | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HINT-04 |
| After a relink that changed catalog contents, hints refresh (`workspace/inlayHint/refresh`) | integration | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HINT-05 |
| Untranslated entry → hint reads ` = (untranslated)`, never bare ` = ` | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HINT-06 |
| Fuzzy entry → hint reads ` = <text> (fuzzy)` | unit | [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) | REQ-HINT-07 |
| Unresolved / unknown msgid → no hint | unit | [fstring-call](../foundations/E17-testing.md#fstring-call), [unknown-msgid](../foundations/E17-testing.md#unknown-msgid) | REQ-HINT-02 |

### 11.3 Fixtures

Reusable fixtures live in the [E17 registry](../foundations/E17-testing.md#5-fixtures-registry) — linked above. This feature defines no fixtures of its own; it reuses [clean-shopfront](../foundations/E17-testing.md#clean-shopfront) for the shown/untranslated/fuzzy/truncation rows and the relink-refresh path, [fstring-call](../foundations/E17-testing.md#fstring-call) for the unresolved case, and [unknown-msgid](../foundations/E17-testing.md#unknown-msgid) for the unknown case.

### 11.4 Requirement coverage

Every load-bearing requirement maps to a test — this table is the proof.

| Requirement | Covered by |
|---|---|
| REQ-HINT-01 | `req_hint_01_no_locale_returns_no_hints` |
| REQ-HINT-02 | `req_hint_02_one_hint_per_resolved_call`, `req_hint_02_unresolved_and_unknown_get_no_hint` |
| REQ-HINT-03 | `req_hint_03_hint_sits_after_call_formatted`, `req_hint_03_only_hints_in_range` |
| REQ-HINT-04 | `req_hint_04_long_translation_truncated` |
| REQ-HINT-05 | `req_hint_05_refreshes_after_relink` |
| REQ-HINT-06 | `req_hint_06_untranslated_is_named` |
| REQ-HINT-07 | `req_hint_07_fuzzy_is_marked` |

## 12. End-to-End Test Plan

Driving the built binary as an editor would, configure a locale, request `textDocument/inlayHint` over a range, and assert the hint labels and the refresh notification over the wire.

### 12.1 Coverage target

**100% of the feature's scope, end to end** — the happy path plus the reasonably possible error paths (no config, an untranslated entry, a fuzzy entry, an over-long translation). See the policy in [E29 §2](../foundations/E29-e2e-testing.md#2-coverage-policy).

### 12.2 Scenarios

Each scenario opens a fixture workspace, sends an `inlayHint` request for the file's range, and asserts the response.

| # | Journey | Path | Expected outcome |
|---|---|---|---|
| E2E-01 | `inlay_hint_locale = "de"`, request hints over `views.py` | happy | `_("Checkout")` carries a hint labeled ` = Kasse` |
| E2E-02 | No `inlay_hint_locale` set, request hints | error | The response is empty — no hints for any call |
| E2E-03 | A translation longer than ~40 chars | error | The hint label is cut to ~40 chars with a trailing `…` |
| E2E-04 | A fuzzy entry (`pgettext("button", "Save")`) | error | The hint label ends with ` (fuzzy)` |
| E2E-05 | Add a `msgstr` in the catalog, then relink | happy | The server sends `workspace/inlayHint/refresh` and a re-request shows the new hint |

### 12.3 Acceptance criteria & Definition of Done

The §12.2 scenarios, written Given/When/Then, are this feature's acceptance criteria:

| # | Given | When | Then |
|---|---|---|---|
| AC-01 | the clean-shopfront workspace is open with `inlay_hint_locale = "de"` | you request inlay hints over `views.py` | `_("Checkout")` carries a hint reading ` = Kasse` |
| AC-02 | no `inlay_hint_locale` is configured | you request inlay hints | no hints are returned for any call |
| AC-03 | a locale has a translation longer than ~40 chars | you request the hint for that call | the label is truncated to ~40 chars with a trailing `…` |
| AC-04 | German's `Save` is `#, fuzzy` | you request the hint for `pgettext("button", "Save")` | the label ends with ` (fuzzy)` |
| AC-05 | a catalog gains a `msgstr` and the index relinks | the relink completes | the server sends `workspace/inlayHint/refresh` and the next request shows the new hint |

**Definition of Done:** every `REQ-HINT-NN` has a passing test (§11.4), every acceptance scenario above passes, and every enabled non-functional concern (§13) is verified.

## 13. Non-Functional Requirements

### 13.1 Security & Privacy

- **Access & validation** — a hint is a read-only render of local catalog data already loaded into the index; it never executes user code, opens a network connection, or shells out (P1).
- **Data sensitivity** — the hint shows only msgids and translations from the user's own workspace; no PII, secrets, or telemetry leave the process.
- **Baseline** — the only untrusted input is catalog/source text, parsed defensively upstream ([F01](F01-catalog-index.md)/[F02](F02-message-extraction.md)); the hint renders the resulting facts and adds no new trust boundary.

## 15. Open Questions & Decisions

- None open.

## 16. Cross-References

- **Depends on:** [F01-catalog-index](F01-catalog-index.md) — the index the hint reads (`lookup`); [F02-message-extraction](F02-message-extraction.md) — the calls hints anchor to; [E15-app-config](../foundations/E15-app-config.md) — the `inlay_hint_locale` key.
- **Related:** [F05-hover](F05-hover.md) — the full card behind a truncated hint; [F12-code-lens](F12-code-lens.md) — coverage and translate actions; [E07-data-model](../foundations/E07-data-model.md) — `TranslationCall`, `CatalogEntry`, `CatalogKey`.
- **Testing:** [E17-testing](../foundations/E17-testing.md) — the coverage policy and the shared fixtures §11 reuses; [E29-e2e-testing](../foundations/E29-e2e-testing.md) — the harness and patterns §12 reuses.

## 17. Changelog

- **2026-06-15** — v0.2: restructured to the updated spec-writer template. Added §6 UI Mockups sketching the dimmed inline hint and its shown/untranslated/fuzzy/disabled states, §7 a request-to-hint flow, §8 the `InlayHint` data shape, §11 Testing (coverage, plan, fixtures, and a per-requirement table mapping REQ-HINT-01..07), §12 End-to-End Test Plan with Given/When/Then acceptance and a DoD, §13.1 Security & Privacy, and §13.2 Accessibility (content-level). Promoted the named-untranslated rule to its own REQ-HINT-06 and renumbered to canonical section order; all prior content preserved.
- **2026-06-15** — Initial draft: opt-in `inlay_hint_locale` previews (REQ-HINT-01/02), the ` = <translation>` placement and ~40-char truncation (REQ-HINT-03/04), `workspace/inlayHint/refresh` after a relink (REQ-HINT-05), and the untranslated/fuzzy/unresolved cases (REQ-HINT-07). Translated from the legacy `features/inlay_hint.rs`.
</content>
</invoke>
