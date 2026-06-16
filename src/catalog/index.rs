use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

// ── CatalogKey ────────────────────────────────────────────────────────────────

/// The lookup key for every catalog feature: msgid + optional msgctxt.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
    pot_entries: HashMap<CatalogKey, CatalogEntry>,
    locales: BTreeSet<String>,
    domains: BTreeSet<String>,
}

#[allow(dead_code)]
impl CatalogIndex {
    /// Build the index from a flat list of loaded entries.
    pub fn build(all_entries: Vec<CatalogEntry>) -> Self {
        let mut entries: HashMap<CatalogKey, Vec<CatalogEntry>> = HashMap::new();
        let mut pot_entries: HashMap<CatalogKey, CatalogEntry> = HashMap::new();
        let mut locales = BTreeSet::new();
        let mut domains = BTreeSet::new();

        for entry in all_entries {
            if !entry.locale.is_empty() {
                locales.insert(entry.locale.clone());
            }
            domains.insert(entry.domain.clone());
            let key = entry.key();
            if entry.locale.is_empty() {
                // .pot template
                pot_entries.insert(key, entry);
            } else {
                entries.entry(key).or_default().push(entry);
            }
        }

        Self {
            entries,
            pot_entries,
            locales,
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
    pub fn missing_locales(&self, key: &CatalogKey) -> Vec<String> {
        let translated: std::collections::HashSet<&str> = self
            .lookup(key)
            .iter()
            .filter(|e| e.msgstr.iter().any(|s| !s.is_empty()))
            .map(|e| e.locale.as_str())
            .collect();
        self.locales
            .iter()
            .filter(|l| !translated.contains(l.as_str()))
            .cloned()
            .collect()
    }

    /// All entries that came from a specific catalog file.
    pub fn entries_for_file(&self, path: &Path) -> Vec<&CatalogEntry> {
        self.entries
            .values()
            .flatten()
            .chain(self.pot_entries.values())
            .filter(|e| e.file_path == path)
            .collect()
    }

    /// The `.pot` template entry for the key, if it exists.
    pub fn lookup_pot(&self, key: &CatalogKey) -> Option<&CatalogEntry> {
        self.pot_entries.get(key)
    }

    /// Whether the key exists in the `.pot` template.
    pub fn is_in_pot(&self, key: &CatalogKey) -> bool {
        self.pot_entries.contains_key(key)
    }

    /// Every key present in the `.pot` template.
    pub fn all_pot_keys(&self) -> impl Iterator<Item = &CatalogKey> {
        self.pot_entries.keys()
    }

    /// Returns `true` if the index has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.pot_entries.is_empty()
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
}
