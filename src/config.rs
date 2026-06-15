pub const BUILTIN_INDICATORS: &[&str] = &["gettext", "_(", "ngettext", "pgettext", "{% trans"];

#[derive(Clone)]
pub struct Config {
    pub extra_keywords: Vec<String>,
    pub jinja_extensions: Vec<String>,
}

impl Config {
    pub fn indicators(&self) -> Vec<String> {
        BUILTIN_INDICATORS
            .iter()
            .map(|s| s.to_string())
            .chain(self.extra_keywords.iter().cloned())
            .collect()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            extra_keywords: vec![],
            jinja_extensions: vec![
                ".html".to_string(),
                ".jinja2".to_string(),
                ".j2".to_string(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indicators_includes_builtins() {
        let cfg = Config::default();
        let inds = cfg.indicators();
        assert!(inds.contains(&"_(".to_string()));
        assert!(inds.contains(&"gettext".to_string()));
        assert!(inds.contains(&"{% trans".to_string()));
    }

    #[test]
    fn indicators_includes_extra_keywords() {
        let cfg = Config {
            extra_keywords: vec!["lazy_gettext".to_string()],
            ..Config::default()
        };
        let inds = cfg.indicators();
        assert!(inds.contains(&"lazy_gettext".to_string()));
    }
}
