# Glossary

> **Status:** Living (continuously maintained)
>
> **Last updated:** 2026-06-15
>
> **Purpose:** The canonical definition of every domain term the suite uses. Defined once here, linked everywhere else.

---

Terms are alphabetical. When a spec introduces a new term, it gets a row here in the same edit.

| Term | Definition |
|---|---|
| **Capability spec** | A feature spec that owns one LSP capability's user-facing behavior across all domains — F03–F10 and F12. F11 and F13 add a capability surface on top of their own domain logic. Contrast with a domain spec and a delivery spec. |
| **Catalog** | A gettext translation file: a `.po` (one locale's translations) or `.pot` (the untranslated template). Parsed by `polib`. Owned by [F01](features/F01-catalog-index.md). |
| **Catalog entry** | One record in a catalog — a msgid with its msgstr(s), optional msgctxt and plural, flags (`fuzzy`, `obsolete`), and source location. The indexed unit. |
| **Catalog index** | The in-memory map from `(msgid, msgctxt)` to every catalog entry that defines it, across all locales and domains. Built in pass 2; the heart of the server. Owned by [F01](features/F01-catalog-index.md). |
| **Catalog key** | The `(msgid, msgctxt)` pair that uniquely identifies a message. Two entries with the same msgid but different context are different keys. |
| **Delivery spec** | A feature spec that packages and ships the server rather than adding a capability — F14 (editor integration), F15 (CLI), F16 (release & CI). Contrast with a domain spec and a capability spec. |
| **Domain** | A gettext text domain — the catalog's base filename (`messages`, `admin`), letting one project ship several independent catalogs. Distinct from a Python domain or web domain. |
| **Domain spec** | A feature spec that owns one domain's indexing semantics — what is extracted and linked: F01, F02. Contrast with a capability spec. |
| **Extraction (pass 1)** | The per-file pass that turns source into facts: translation calls from Python and Jinja, entries from catalogs. Error-tolerant and position-accurate. Owned by [F02](features/F02-message-extraction.md). |
| **Fact** | A single raw extraction from pass 1 — one translation call or one catalog entry — before it is linked to anything. |
| **Fuzzy** | A catalog entry flagged `#, fuzzy`: a translation that exists but is unverified, usually because the msgid changed. Gettext ignores fuzzy translations at runtime. |
| **gettext variant** | One of the translation functions babel-lsp recognizes — `_`, `gettext`, `ngettext` (plural), `pgettext` (context), `dgettext` (domain), their `npgettext`/`dngettext` combinations, and `*_lazy`/`u*` forms. Each has a known argument layout. |
| **Linking (pass 2)** | The debounced workspace pass that builds the catalog index from facts and resolves each source msgid to its catalog entries. |
| **Locale** | A language/region identifier (`de`, `fr`, `pt_BR`) naming a translation target. Each locale owns one `.po` per domain, conventionally at `<locale>/LC_MESSAGES/<domain>.po`. |
| **Msgctxt** | The optional disambiguating context of a message, set by `pgettext("button", "Save")`. Part of the catalog key. |
| **Msgid** | The source string that identifies a message — the first string argument to a translation call, and the `msgid` line in a catalog. The lookup key translators work against. |
| **Msgstr** | The translated string in a catalog entry. Plural messages carry several — `msgstr[0]`, `msgstr[1]`, … — one per plural form. |
| **Obsolete** | A catalog entry marked `#~`: a translation kept for reference whose msgid no longer appears in the template. |
| **Placeholder** | A format token inside a msgid that must survive translation unchanged — printf-style `%(num)d`/`%s` or brace-style `{name}`. A mismatch between msgid and msgstr is a diagnostic. |
| **Plural-Forms** | The catalog-header expression declaring how many plural forms a locale has and how to choose one — `nplurals=2; plural=(n != 1);`. Governs how many `msgstr[i]` an entry needs. |
| **POT template** | The `.pot` catalog: every msgid the source uses, with empty translations. The reference for what *should* be translated and the source of truth for obsolete detection. |
| **Translation call** | A recognized gettext-variant call in source — `_("Checkout")`, `ngettext(...)`, a `{% trans %}` block — carrying its msgid, optional plural/context/domain, and source ranges. The source-side fact. |
| **Unresolved msgid** | The state of a translation call whose msgid can't be read statically (a variable or f-string first argument). Kept for reporting, excluded from msgid lookups, per constitution P4. |
| **Unsaved overlay** | The rule that an open, unsaved catalog buffer shadows its on-disk version in the index, so features reflect the editor's current text. |
| **Workspace state** | The server's whole in-memory model: open documents with their parse trees, the catalog index, the resolved config, and the workspace root. Defined in [E07-data-model](foundations/E07-data-model.md). |

## Changelog

- **2026-06-15** — Added **Delivery spec** and clarified the **Capability spec** / **Domain spec** contrast so every feature spec maps to exactly one category (F11/F13 noted as blends).
- **2026-06-15** — Initial glossary: catalog and gettext domain vocabulary, the two-pass terms, and the indexing types.
