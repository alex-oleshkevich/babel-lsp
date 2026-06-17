use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ── CatalogKey ────────────────────────────────────────────────────────────────

/// The lookup key for every catalog feature: msgid + optional msgctxt.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CatalogKey {
    pub msgid: String,
    pub msgctxt: Option<String>,
}

#[allow(dead_code)]
impl CatalogKey {
    pub fn new(msgid: impl Into<String>) -> Self {
        Self {
            msgid: msgid.into(),
            msgctxt: None,
        }
    }

    pub fn with_ctx(msgid: impl Into<String>, msgctxt: impl Into<String>) -> Self {
        Self {
            msgid: msgid.into(),
            msgctxt: Some(msgctxt.into()),
        }
    }
}

// ── CatalogEntry ──────────────────────────────────────────────────────────────

/// One parsed message from a `.po` or `.pot` catalog.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct EntryFlags {
    pub fuzzy: bool,
    pub obsolete: bool,
}

/// One parsed catalog entry, carrying everything features need.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct CatalogEntry {
    pub locale: String,
    pub domain: String,
    pub msgid: String,
    pub msgctxt: Option<String>,
    pub msgid_plural: Option<String>,
    pub msgstr: Vec<String>,
    pub flags: EntryFlags,
    pub file_path: PathBuf,
    pub line: u32,
}

#[allow(dead_code)]
impl CatalogEntry {
    pub fn key(&self) -> CatalogKey {
        CatalogKey {
            msgid: self.msgid.clone(),
            msgctxt: self.msgctxt.clone(),
        }
    }
}

// ── CatalogIndex ─────────────────────────────────────────────────────────────

/// The in-memory join table: one lookup answers "everything known about this msgid".
#[allow(dead_code)]
pub struct CatalogIndex {
    entries: HashMap<CatalogKey, Vec<CatalogEntry>>,
    /// Keyed by (domain, CatalogKey) so that the same msgid in two different
    /// domains does not overwrite each other.
    pot_entries: HashMap<(String, CatalogKey), CatalogEntry>,
    /// Header entries (msgid "") keyed by file path, kept separate so that
    /// all_msgids() does not expose the empty key and proj/unused-id does not
    /// fire on the catalog header.
    header_entries: HashMap<PathBuf, CatalogEntry>,
    locales: BTreeSet<String>,
    locales_by_domain: HashMap<String, BTreeSet<String>>,
    domains: BTreeSet<String>,
}

#[allow(dead_code)]
impl CatalogIndex {
    /// Build the index from a flat list of loaded entries.
    pub fn build(all_entries: Vec<CatalogEntry>) -> Self {
        let mut entries: HashMap<CatalogKey, Vec<CatalogEntry>> = HashMap::new();
        let mut pot_entries: HashMap<(String, CatalogKey), CatalogEntry> = HashMap::new();
        let mut header_entries: HashMap<PathBuf, CatalogEntry> = HashMap::new();
        let mut locales = BTreeSet::new();
        let mut locales_by_domain: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut domains = BTreeSet::new();

        for entry in all_entries {
            if !entry.locale.is_empty() {
                locales.insert(entry.locale.clone());
                locales_by_domain
                    .entry(entry.domain.clone())
                    .or_default()
                    .insert(entry.locale.clone());
            }
            domains.insert(entry.domain.clone());
            let key = entry.key();
            if entry.msgid.is_empty() {
                // Catalog header entry — store separately so all_msgids() does
                // not expose the empty key and proj/unused-id never fires on it.
                header_entries
                    .entry(entry.file_path.clone())
                    .or_insert(entry);
            } else if entry.locale.is_empty() {
                // .pot template — include domain in key to avoid cross-domain collisions
                pot_entries.insert((entry.domain.clone(), key), entry);
            } else {
                entries.entry(key).or_default().push(entry);
            }
        }

        Self {
            entries,
            pot_entries,
            header_entries,
            locales,
            locales_by_domain,
            domains,
        }
    }

    /// All entries for a msgid across all locales and domains.
    pub fn lookup(&self, key: &CatalogKey) -> &[CatalogEntry] {
        self.entries.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Every distinct key in the index.
    pub fn all_msgids(&self) -> impl Iterator<Item = &CatalogKey> {
        self.entries.keys()
    }

    /// Every locale seen across all loaded catalogs.
    pub fn all_locales(&self) -> &BTreeSet<String> {
        &self.locales
    }

    /// Every domain seen across all loaded catalogs.
    pub fn all_domains(&self) -> &BTreeSet<String> {
        &self.domains
    }

    /// Locales that have no translation or an empty msgstr for `key`.
    ///
    /// Missing-locale computation is scoped per domain: a locale is considered
    /// missing only when it exists in a domain that carries `key` but has no
    /// non-empty msgstr for it in that domain.  This prevents a translation in
    /// domain 'admin' from masking a missing translation for the same msgid in
    /// domain 'messages'.
    pub fn missing_locales(&self, key: &CatalogKey) -> Vec<String> {
        let all_entries = self.lookup(key);

        // Collect the set of domains that have at least one entry for this key.
        let mut domains_with_key: std::collections::HashSet<&str> =
            std::collections::HashSet::new();
        for e in all_entries {
            domains_with_key.insert(e.domain.as_str());
        }

        let mut missing: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for domain in &domains_with_key {
            let domain_locales = match self.locales_by_domain.get(*domain) {
                Some(ls) => ls,
                None => continue,
            };
            // Build translated set scoped to this domain.
            let translated: std::collections::HashSet<&str> = all_entries
                .iter()
                .filter(|e| e.domain.as_str() == *domain)
                .filter(|e| e.msgstr.iter().any(|s| !s.is_empty()))
                .map(|e| e.locale.as_str())
                .collect();
            for locale in domain_locales {
                if !translated.contains(locale.as_str()) {
                    missing.insert(locale.clone());
                }
            }
        }

        missing.into_iter().collect()
    }

    /// All entries that came from a specific catalog file, including the header.
    pub fn entries_for_file(&self, path: &Path) -> Vec<&CatalogEntry> {
        let header = self.header_entries.get(path).into_iter();
        self.entries
            .values()
            .flatten()
            .chain(self.pot_entries.values())
            .chain(header)
            .filter(|e| e.file_path == path)
            .collect()
    }

    /// The `.pot` template entry for the key, if it exists.
    ///
    /// When the same msgid is present in multiple domains, returns the entry
    /// with the lexicographically smallest file path for determinism.
    pub fn lookup_pot(&self, key: &CatalogKey) -> Option<&CatalogEntry> {
        self.pot_entries
            .iter()
            .filter(|((_, k), _)| k == key)
            .map(|(_, e)| e)
            .min_by_key(|e| &e.file_path)
    }

    /// Whether the key exists in any `.pot` template.
    pub fn is_in_pot(&self, key: &CatalogKey) -> bool {
        self.pot_entries.keys().any(|(_, k)| k == key)
    }

    /// Every distinct key present in the `.pot` templates.
    ///
    /// Duplicates across domains are deduplicated; order is unspecified.
    pub fn all_pot_keys(&self) -> impl Iterator<Item = &CatalogKey> {
        // The HashMap guarantees that each (domain, CatalogKey) pair is unique,
        // but the same CatalogKey may appear under different domains.  We deduplicate
        // by returning only the first occurrence of each CatalogKey value.
        let mut seen: std::collections::HashSet<&CatalogKey> = std::collections::HashSet::new();
        self.pot_entries
            .keys()
            .map(|(_, k)| k)
            .filter(move |k| seen.insert(k))
    }

    /// Returns `true` if the index has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.pot_entries.is_empty()
    }

    /// Returns the lexicographically smallest `.pot` template file path.
    ///
    /// Using the minimum path makes the result deterministic regardless of
    /// HashMap iteration order or how many `.pot` files are loaded.
    pub fn pot_file_path(&self) -> Option<&Path> {
        self.pot_entries
            .values()
            .map(|e| e.file_path.as_path())
            .min()
    }

    /// Returns all unique `.po` locale file paths (sorted, deduplicated).
    pub fn po_file_paths(&self) -> Vec<&Path> {
        let mut paths: Vec<&Path> = self
            .entries
            .values()
            .flatten()
            .map(|e| e.file_path.as_path())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths
    }

    /// Returns `true` if at least one `.pot` template entry was loaded.
    ///
    /// Used by `po/obsolete`: without a `.pot` in the workspace, "absent from
    /// template" is unprovable, so the check stays silent (REQ-DIAG-09).
    pub fn has_pot_entries(&self) -> bool {
        !self.pot_entries.is_empty()
    }
}

impl Default for CatalogIndex {
    fn default() -> Self {
        Self::build(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(locale: &str, msgid: &str, msgstr: &str) -> CatalogEntry {
        CatalogEntry {
            locale: locale.into(),
            domain: "messages".into(),
            msgid: msgid.into(),
            msgctxt: None,
            msgid_plural: None,
            msgstr: vec![msgstr.into()],
            flags: EntryFlags {
                fuzzy: false,
                obsolete: false,
            },
            file_path: PathBuf::from("/locale/messages.po"),
            line: 1,
        }
    }

    // --- CatalogKey ---

    #[test]
    fn catalog_key_equality() {
        let a = CatalogKey::new("Checkout");
        let b = CatalogKey::new("Checkout");
        assert_eq!(a, b);
    }

    #[test]
    fn catalog_key_distinguishes_context() {
        let a = CatalogKey::with_ctx("Save", "button");
        let b = CatalogKey::with_ctx("Save", "menu");
        assert_ne!(a, b);
    }

    #[test]
    fn catalog_key_no_context_ne_with_context() {
        let a = CatalogKey::new("Save");
        let b = CatalogKey::with_ctx("Save", "button");
        assert_ne!(a, b);
    }

    #[test]
    fn entry_key_round_trips() {
        let entry = CatalogEntry {
            locale: "de".into(),
            domain: "messages".into(),
            msgid: "Checkout".into(),
            msgctxt: Some("ecommerce".into()),
            msgid_plural: None,
            msgstr: vec!["Kasse".into()],
            flags: EntryFlags {
                fuzzy: false,
                obsolete: false,
            },
            file_path: "/locale/de/LC_MESSAGES/messages.po".into(),
            line: 10,
        };
        let key = entry.key();
        assert_eq!(key.msgid, "Checkout");
        assert_eq!(key.msgctxt.as_deref(), Some("ecommerce"));
    }

    // --- CatalogIndex ---

    #[test]
    fn build_groups_po_entries_by_key() {
        let entries = vec![
            make_entry("de", "Checkout", "Kasse"),
            make_entry("fr", "Checkout", "Caisse"),
        ];
        let idx = CatalogIndex::build(entries);
        let key = CatalogKey::new("Checkout");
        assert_eq!(idx.lookup(&key).len(), 2);
    }

    #[test]
    fn build_separates_pot_from_po() {
        let mut pot = make_entry("", "Checkout", "");
        pot.file_path = "/locale/messages.pot".into();
        let po = make_entry("de", "Checkout", "Kasse");
        let idx = CatalogIndex::build(vec![pot, po]);
        let key = CatalogKey::new("Checkout");
        assert!(idx.is_in_pot(&key));
        assert_eq!(idx.lookup(&key).len(), 1); // only .po entry
    }

    #[test]
    fn all_locales_collected() {
        let entries = vec![
            make_entry("de", "Checkout", "Kasse"),
            make_entry("fr", "Checkout", "Caisse"),
        ];
        let idx = CatalogIndex::build(entries);
        let locales = idx.all_locales();
        assert!(locales.contains("de"));
        assert!(locales.contains("fr"));
    }

    #[test]
    fn pot_locale_not_in_locales_set() {
        let mut pot = make_entry("", "Checkout", "");
        pot.file_path = "/locale/messages.pot".into();
        let idx = CatalogIndex::build(vec![pot]);
        assert!(!idx.all_locales().contains(""));
    }

    #[test]
    fn missing_locales_finds_untranslated() {
        let entries = vec![
            make_entry("de", "Checkout", "Kasse"),
            make_entry("fr", "Checkout", ""), // empty → missing
        ];
        let idx = CatalogIndex::build(entries);
        let missing = idx.missing_locales(&CatalogKey::new("Checkout"));
        assert!(missing.contains(&"fr".to_string()));
        assert!(!missing.contains(&"de".to_string()));
    }

    #[test]
    fn missing_locales_scoped_per_domain() {
        // "Save" is translated in domain 'admin' for 'de', but absent/empty in
        // domain 'messages' for 'de'.  The old code would report 'de' as NOT
        // missing because it found a translation in another domain.
        let mut admin_de = make_entry("de", "Save", "Speichern");
        admin_de.domain = "admin".into();
        admin_de.file_path = "/locale/de/admin.po".into();

        let mut messages_de = make_entry("de", "Save", "");
        messages_de.domain = "messages".into();

        let idx = CatalogIndex::build(vec![admin_de, messages_de]);
        let missing = idx.missing_locales(&CatalogKey::new("Save"));
        // 'de' must appear: it is untranslated in domain 'messages'.
        assert!(
            missing.contains(&"de".to_string()),
            "de should be missing in domain 'messages' even if translated in 'admin'"
        );
    }

    #[test]
    fn lookup_unknown_key_returns_empty() {
        let idx = CatalogIndex::build(vec![]);
        assert!(idx.lookup(&CatalogKey::new("nonexistent")).is_empty());
    }

    #[test]
    fn is_empty_on_fresh_index() {
        assert!(CatalogIndex::default().is_empty());
    }

    #[test]
    fn entries_for_file_filters_by_path() {
        let mut e1 = make_entry("de", "Checkout", "Kasse");
        e1.file_path = PathBuf::from("/locale/de/LC_MESSAGES/messages.po");
        let mut e2 = make_entry("fr", "Checkout", "Caisse");
        e2.file_path = PathBuf::from("/locale/fr/LC_MESSAGES/messages.po");
        let idx = CatalogIndex::build(vec![e1, e2]);
        let de_path = Path::new("/locale/de/LC_MESSAGES/messages.po");
        assert_eq!(idx.entries_for_file(de_path).len(), 1);
    }

    #[test]
    fn pot_file_path_returns_pot_entry_path() {
        let mut pot = make_entry("", "Checkout", "");
        pot.file_path = PathBuf::from("/locale/messages.pot");
        let idx = CatalogIndex::build(vec![pot]);
        assert_eq!(idx.pot_file_path(), Some(Path::new("/locale/messages.pot")));
    }

    #[test]
    fn pot_file_path_none_when_no_pot() {
        let idx = CatalogIndex::build(vec![make_entry("de", "Checkout", "Kasse")]);
        assert!(idx.pot_file_path().is_none());
    }

    #[test]
    fn po_file_paths_returns_unique_paths() {
        let mut e1 = make_entry("de", "Checkout", "Kasse");
        e1.file_path = PathBuf::from("/locale/de/LC_MESSAGES/messages.po");
        let mut e2 = make_entry("de", "Save", "Speichern");
        e2.file_path = PathBuf::from("/locale/de/LC_MESSAGES/messages.po");
        let mut e3 = make_entry("fr", "Checkout", "Caisse");
        e3.file_path = PathBuf::from("/locale/fr/LC_MESSAGES/messages.po");
        let idx = CatalogIndex::build(vec![e1, e2, e3]);
        let paths = idx.po_file_paths();
        assert_eq!(paths.len(), 2, "should be deduplicated");
    }

    #[test]
    fn po_file_paths_empty_when_no_po_entries() {
        let idx = CatalogIndex::build(vec![]);
        assert!(idx.po_file_paths().is_empty());
    }

    #[test]
    fn pot_entries_cross_domain_no_collision() {
        // Same msgid in two different domains — both must survive.
        let mut pot_a = make_entry("", "Save", "");
        pot_a.domain = "messages".into();
        pot_a.file_path = PathBuf::from("/locale/messages.pot");

        let mut pot_b = make_entry("", "Save", "");
        pot_b.domain = "admin".into();
        pot_b.file_path = PathBuf::from("/locale/admin.pot");

        let idx = CatalogIndex::build(vec![pot_a, pot_b]);

        // The key must be found
        assert!(idx.is_in_pot(&CatalogKey::new("Save")));
        // Both domains preserved: pot_entries has 2 entries
        assert_eq!(idx.pot_entries.len(), 2);
    }

    #[test]
    fn pot_file_path_deterministic_with_multiple_pots() {
        let mut pot_a = make_entry("", "Save", "");
        pot_a.domain = "admin".into();
        pot_a.file_path = PathBuf::from("/locale/admin.pot");

        let mut pot_b = make_entry("", "Checkout", "");
        pot_b.domain = "messages".into();
        pot_b.file_path = PathBuf::from("/locale/messages.pot");

        let idx = CatalogIndex::build(vec![pot_a, pot_b]);
        // min by path: /locale/admin.pot < /locale/messages.pot
        assert_eq!(idx.pot_file_path(), Some(Path::new("/locale/admin.pot")));
    }
}
