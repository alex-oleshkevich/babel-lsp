# F07 — Code Actions

> **Status:** Draft
>
> **Version:** 0.2   ·   **Last updated:** 2026-06-15
>
> **Purpose:** Quick fixes on catalog entries — fill an empty translation, toggle fuzzy, repair a placeholder, add missing plural forms — as one-click WorkspaceEdits.
>
> **Depends on:** [F03-diagnostics](F03-diagnostics.md), [E07-data-model](../foundations/E07-data-model.md)   ·   **Related:** [F11-hardcoded-strings](F11-hardcoded-strings.md), [F13-catalog-commands](F13-catalog-commands.md)

> Requirement tag: **ACT**

---

## 1. Purpose & Scope

A diagnostic tells you a `.po` entry is wrong. This spec is the other half: it offers the edit that fixes it, one click away.

Every action here is a standard `CodeAction` carrying a `WorkspaceEdit`. The client applies the edit itself — there is no `workspace/executeCommand` round-trip and no server-side state to wait on.

This spec covers:

- Copy msgid → msgstr for an empty entry
- Mark / remove the `#, fuzzy` flag
- Fix a placeholder mismatch by copying the msgid
- Add the missing plural `msgstr[i]` forms for the locale
- The batch "Copy msgid" across a multi-entry selection

## 2. Non-Goals / Out of Scope

- Project-wide `pybabel` operations — extract, update, compile catalogs — owned by [F13-catalog-commands](F13-catalog-commands.md). Those are commands, not entry-local edits.
- Detecting a hardcoded source string worth extracting — [F11-hardcoded-strings](F11-hardcoded-strings.md) owns the source-side detection. The edit that wraps the literal and appends the `.pot` entry lives here, but F11 decides when to offer it.
- Sorting, header normalization, and catalog-to-template sync — whole-file restructurings, not quick fixes; tracked with the catalog commands in [F13](F13-catalog-commands.md).

## 3. Detailed Specification

### 3.1 How an action is shaped

Each action below is a `CodeActionKind::QUICKFIX` carrying a `WorkspaceEdit` whose `changes` map names the `.po` URI and a list of `TextEdit`s. No command, no follow-up request.

**REQ-ACT-01 — Quick fixes return a WorkspaceEdit, never a command.**

The handler computes the precise text edit and attaches it to the action. When the user picks the action, the editor applies the `WorkspaceEdit` directly. The server is not called again, so the fix never races a later edit on the server side.

**REQ-ACT-02 — A fix is offered only when its edit is provably correct.**

Constitution P4 governs edits even more strictly than squiggles: a wrong edit corrupts the catalog. Each action states its precondition; when it fails, the action simply doesn't appear. The server never guesses a translation — copying the msgid is a scaffold the translator overwrites, not an answer.

### 3.2 Pairing with F03 diagnostics

The quick fixes mirror the [F03](F03-diagnostics.md) diagnostic catalog. Each attaches to its diagnostic's range so the editor shows the lightbulb on the squiggle.

| Diagnostic ([F03](F03-diagnostics.md)) | Action | Edit |
|---|---|---|
| `po/missing-translation` | Copy msgid to msgstr | Fill the empty `msgstr` with the msgid text |
| `po/fuzzy` | Remove fuzzy flag | Drop `fuzzy` from the `#,` flags line |
| (any translated entry) | Mark as fuzzy | Add `#, fuzzy` above the entry |
| `po/format-mismatch` | Fix placeholder mismatch | Copy the msgid into the `msgstr` |
| `po/plural-count` | Add missing plural forms | Emit the missing `msgstr[i]` lines |

**REQ-ACT-03 — Fixes read the diagnostic's data payload, then fall back to the snapshot.**

When the client supports `publishDiagnostics.dataSupport`, F03 attaches the entry's resolved facts to the diagnostic's `data` field, and the action reads them directly. When `data` is absent, the handler recomputes the same facts from the workspace snapshot — it locates the entry whose range contains the cursor and re-derives the fix. The payload is an optimization, not a dependency.

### 3.3 Copy msgid → msgstr

For an empty entry, the most common fix is to seed the translation with the source text, then edit it. The shopfront's French `"Checkout"` has no translation yet; on that entry the server offers **"Copy msgid to msgstr"**.

**REQ-ACT-04 — Copy msgid fills every empty msgstr slot.**

The action fires when every `msgstr` slot of the entry is empty. For a singular entry it writes one `msgstr`; for a plural entry it writes `msgstr[0]` from the msgid and each further `msgstr[i]` from `msgid_plural`, up to the locale's `nplurals` (§3.6). The msgid text is gettext-escaped before it lands in the quotes (§3.7).

```po
# locale/fr/LC_MESSAGES/messages.po — was
msgid "Checkout"
msgstr ""

# becomes
msgid "Checkout"
msgstr "Checkout"
```

### 3.4 Mark / remove fuzzy

A fuzzy entry has a translation that gettext ignores until a human verifies it. The shopfront's German `"Save"` is marked `#, fuzzy`; on it the server offers **"Remove fuzzy flag"**.

**REQ-ACT-05 — Fuzzy toggling rewrites only the flags line.**

*Remove* fires on a fuzzy entry. When `fuzzy` is the only flag, the whole `#, fuzzy` line is deleted; when other flags remain (`python-format`), the line is rewritten without `fuzzy`. *Mark* fires on a non-obsolete, non-header entry that has a non-empty translation: an existing flags line gains `fuzzy` at the front, or a new `#, fuzzy` line is inserted above the msgid.

```po
# locale/de/LC_MESSAGES/messages.po — was
#, fuzzy
msgid "Save"
msgstr "Sichern"

# becomes (Remove fuzzy flag)
msgid "Save"
msgstr "Sichern"
```

### 3.5 Fix placeholder mismatch

When a `msgstr` drops or renames a placeholder the msgid carries — `%(num)d` becoming `%(naam)d` — gettext-formatted output breaks at runtime. The safe repair the server can prove correct is to copy the msgid wholesale, restoring its placeholders for the translator to re-translate around.

**REQ-ACT-06 — The placeholder fix copies the msgid into the offending msgstr.**

The action fires when a non-empty `msgstr` mismatches its source's placeholder set (the same check F03 runs for `po/format-mismatch`). For a plural entry, the source is the msgid for `msgstr[0]` and the msgid_plural for later forms. The fix replaces the `msgstr` text with the escaped source string. It restores the placeholders, not the translation — the translator still does the language work.

### 3.6 Add missing plural forms

A locale declares how many plural forms it has in its `Plural-Forms` header — `nplurals=3` for Polish, `nplurals=2` for German. An entry with fewer `msgstr[i]` slots than that is incomplete.

**REQ-ACT-07 — Missing plural forms are generated from the locale's nplurals.**

The action fires on a plural entry (one with `msgid_plural`) whose `msgstr` count is below the `nplurals` parsed from the catalog's `Plural-Forms` header. It inserts empty `msgstr[i]` lines for each missing index, after the last existing `msgstr`:

```po
# Polish messages.po (nplurals=3) — was
msgid "%(num)d item"
msgid_plural "%(num)d items"
msgstr[0] "%(num)d przedmiot"
msgstr[1] "%(num)d przedmioty"

# becomes — the missing third form is scaffolded empty
msgstr[2] ""
```

The new slots are left empty, not copied — the right plural wording is a per-form judgment the server can't make (P4).

### 3.7 PO-edit mechanics

The edits above are textual, so two things must be exact: where the edit lands, and how the string is quoted.

**REQ-ACT-08 — Ranges and escaping come from one PO-edit utility.**

A small parser walks the buffer into per-entry spans, recording the line of each `msgid`, `msgstr`, and `#,` flags line, plus multi-line continuations. The fix builders ask that span for the precise replace range — the `msgstr` lines, or the flags line — so an edit never disturbs a neighbouring entry.

Every string written into a `.po` is gettext-escaped first: backslash, double-quote, newline, and tab become `\\`, `\"`, `\n`, `\t`. A msgid containing a quote would otherwise produce an unparseable entry.

```rust
// src/util/po_edit.rs — the contract the builders rely on
pub fn parse_entry_spans(content: &str) -> Vec<PoEntrySpan>;
pub fn span_at_line(spans: &[PoEntrySpan], line: u32) -> Option<&PoEntrySpan>;
pub fn msgstr_replace_range(span: &PoEntrySpan, lines: &[&str]) -> Range;
pub fn flags_line_range(span: &PoEntrySpan, lines: &[&str]) -> Option<Range>;
pub fn escape_po(s: &str) -> String;
```

Position offsets honour the negotiated encoding ([E01 REQ-ARCH-09](../foundations/E01-architecture.md)) — translated strings are full of multi-byte characters, so a miscounted column lands a fix mid-character.

### 3.8 Batch copy across a selection

When the user selects several untranslated entries at once, copying them one by one is tedious. A single batch action covers the lot.

**REQ-ACT-09 — A multi-entry selection offers one batch copy action.**

When the selected range covers more than one empty, non-obsolete entry, the server offers **"Copy msgid to all empty msgstr (N entries)"** — one `WorkspaceEdit` holding a `TextEdit` per entry. The per-entry rule from §3.3 applies to each. The action carries `CodeActionKind::SOURCE` so editors file it under source actions, not the per-entry lightbulb.

## 4. Examples & Use Cases

A translator opens the shopfront's German catalog. The `"Save"` entry wears a `#, fuzzy` flag and a finished translation; they verified it, so they hit the lightbulb and pick **Remove fuzzy flag** — the `#, fuzzy` line vanishes and gettext now serves the translation.

They switch to the French catalog. `"Checkout"` sits empty under `po/missing-translation`. The lightbulb offers **Copy msgid to msgstr**; they accept, `msgstr "Checkout"` appears, and they replace it with `"Commander"`. Selecting the next ten empty entries, they take **Copy msgid to all empty msgstr (10 entries)** to scaffold the batch in one edit.

## 5. Edge Cases & Failure Modes

- Action requested on a non-PO file (a `.py` or template) → no catalog-entry actions; the source-side extract fix from [F11](F11-hardcoded-strings.md) may still apply.
- Plural entry but the `Plural-Forms` header is missing or unparseable → the count is unknown, so **Add missing plural forms** is withheld rather than guessing a default. (Whole-catalog header repair is [F13](F13-catalog-commands.md)'s job.)
- Cursor on the header entry (empty msgid) → no copy or fuzzy action; the header is not a translatable message.
- Obsolete entry (`#~`) → no fuzzy or copy action; an obsolete entry is reference-only.
- A msgstr already matching the msgid's placeholders → `po/format-mismatch` never fired, so the fix isn't offered.
- Selection covering zero or one empty entry → no batch action; the per-entry copy covers the single case.

## 6. Open Questions & Decisions

- **Decision (resolves OQ-ACT-1)** — **Copy msgid to msgstr** is offered only on an *empty* entry, never on a fuzzy non-empty one. A fuzzy translation, however dubious, is a human's work; silently overwriting it with the source text is hostile. On a fuzzy entry the server offers **Remove fuzzy flag** instead, and the translator can clear the text by hand if they truly want to start over.
- **Decision** — Mismatched placeholders are repaired by copying the msgid, not by surgically patching the offending token. Surgical patching needs alignment heuristics that P4 forbids; the wholesale copy is provably placeholder-correct.

## Data Shapes & Code Map

```rust
// src/features/code_action.rs
pub fn code_actions_for_po(
    params: &CodeActionParams,
    content: &str,
    entries: &[&CatalogEntry],
    index: &CatalogIndex,
) -> Vec<CodeAction>;
```

The handler parses the buffer into spans once, then for each entry the cursor touches, tests each fix's precondition (§3.2) and pushes the actions that pass. Files: `features/code_action.rs` (the builders and their gates), `util/po_edit.rs` (spans, ranges, escaping), `util/plural.rs` (`nplurals` parsing).

## 7. Cross-References

- **Depends on:** [F03-diagnostics](F03-diagnostics.md) — the diagnostic codes each fix pairs with and the `data` payload it reads; [E07-data-model](../foundations/E07-data-model.md) — `CatalogEntry`, `CatalogIndex`, and `Plural-Forms`.
- **Related:** [F11-hardcoded-strings](F11-hardcoded-strings.md) — owns extract-message detection; this spec hosts the edit. [F13-catalog-commands](F13-catalog-commands.md) — `pybabel` and whole-catalog operations. [E01-architecture](../foundations/E01-architecture.md) — position encoding for exact ranges; pure-function feature dispatch.
- **Testing:** [E17 §2.5](../foundations/E17-testing.md) — this feature's row in the e2e coverage matrix.

## 8. Changelog

- **2026-06-15** — v0.2: resolved OQ-ACT-1 — Copy msgid to msgstr is offered only on empty entries, never overwriting a fuzzy translation (a fuzzy entry gets Remove fuzzy instead).
- **2026-06-15** — Initial draft: copy-msgid, fuzzy toggle, placeholder-fix, plural-form, and batch-copy quick fixes as direct WorkspaceEdits; F03 pairing with data-payload-then-snapshot fallback; PO-edit range and escaping mechanics; missing-Plural-Forms and non-PO edge cases.
