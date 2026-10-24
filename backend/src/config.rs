//! `steam-mp-launcher.conf`: parse, read with env precedence, write a key.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::paths::atomic_write;

#[derive(Clone, Debug)]
pub struct Config {
    pub path: PathBuf,
    values: BTreeMap<String, String>,
}

pub fn parse(text: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let (key, value) = line.split_once('=').unwrap_or_default();
        let key: String = key.chars().filter(|c| !c.is_whitespace()).collect();
        if key.is_empty() {
            continue;
        }
        let mut value = value.trim().to_owned();
        for quote in ['"', '\''] {
            if value.ends_with(quote) {
                value.pop();
            }
            if value.starts_with(quote) {
                value.remove(0);
            }
        }
        values.insert(key, value);
    }
    values
}

impl Config {
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let values = fs::read_to_string(&path)
            .map(|text| parse(&text))
            .unwrap_or_default();
        Self { path, values }
    }

    pub fn get(&self, key: &str, default: &str) -> String {
        if let Ok(value) = env::var(key) {
            if !value.is_empty() {
                return value;
            }
        }
        self.values
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_owned())
    }

    pub fn get_int(&self, key: &str, default: i64) -> i64 {
        let value = self.get(key, "");
        if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
            value.parse().unwrap_or(default)
        } else {
            default
        }
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let old = fs::read_to_string(&self.path).unwrap_or_default();
        let text = set_text(&old, key, value);
        atomic_write(&self.path, &text).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.values.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    /// Re-read the file: a window that stays open must not keep serving a
    /// stale copy after the conf was edited elsewhere.
    pub fn refresh(&mut self) {
        self.values = fs::read_to_string(&self.path)
            .map(|text| parse(&text))
            .unwrap_or_default();
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// `conf_set`: replace every assignment of the key, append when absent.
pub fn set_text(text: &str, key: &str, value: &str) -> String {
    let mut out = String::with_capacity(text.len() + 32);
    let mut found = false;
    for line in text.split('\n') {
        let probe = line.trim_end_matches('\r');
        if !probe.is_empty() && !probe.starts_with('#') {
            if let Some((left, _)) = probe.split_once('=') {
                if left
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    == key
                {
                    out.push_str(&format!("{key}={value}\n"));
                    found = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !out.is_empty() && out.ends_with("\n\n") {
        out.pop();
    }
    if !found {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("{key}={value}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_keeps_comments_and_replaces_all_duplicates() {
        let text = "# head\nA=1\n\nB=2\nA=3\n";
        let out = set_text(text, "A", "9");
        assert!(out.contains("# head\nA=9\n\nB=2\nA=9\n"));
    }

    #[test]
    fn set_appends_missing_key() {
        let out = set_text("A=1\n", "B", "2");
        assert!(out.contains("A=1\nB=2\n"));
    }

    #[test]
    fn parses_quotes_and_crlf() {
        let parsed = parse("# c\r\nKEY = \"value\"\r\nOTHER='x'\r\n");
        assert_eq!(parsed.get("KEY").unwrap(), "value");
        assert_eq!(parsed.get("OTHER").unwrap(), "x");
    }

    #[test]
    fn refresh_picks_up_an_external_write() {
        let dir = std::env::temp_dir().join(format!(
            "mp-conf-refresh-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("steam-mp-launcher.conf");
        std::fs::write(&path, "PROTON=/old\n").unwrap();
        let mut config = Config::load(&path);
        assert_eq!(config.get("PROTON", ""), "/old");
        // another editor overwrites the file while this window is open
        std::fs::write(&path, "PROTON=/new\n").unwrap();
        config.refresh();
        assert_eq!(config.get("PROTON", ""), "/new");
        std::fs::remove_dir_all(dir).ok();
    }
}
