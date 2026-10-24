//! `launchers.list` / `extras.list`: "label|path", comments, atomic writes.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::paths::{atomic_write, home};

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub label: String,
    pub path: String,
}

pub fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix('~') {
        let home = home();
        format!("{}{}", home.to_string_lossy().trim_end_matches('/'), rest)
    } else {
        path.to_owned()
    }
}

pub fn parse_entries(text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r').trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((label, path)) = line.split_once('|') else {
            continue;
        };
        let label = label.trim();
        let path = expand_home(path.trim());
        if !label.is_empty() && !path.is_empty() {
            entries.push(Entry {
                label: label.to_owned(),
                path,
            });
        }
    }
    entries
}

pub fn read_entries(path: &Path) -> Vec<Entry> {
    fs::read_to_string(path)
        .map(|text| parse_entries(&text))
        .unwrap_or_default()
}

pub fn text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

pub fn write(path: &Path, text: &str) -> Result<(), String> {
    atomic_write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn list_header(path: &Path) -> String {
    let extras = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("extras"))
        .unwrap_or(false);
    let head = if extras {
        "# steam-mp-launcher - extra programs (run alongside)"
    } else {
        "# steam-mp-launcher"
    };
    format!(
        "{head}\n#\n# One per line:   <label>|<path to the .exe>\n\
         # Blank lines and lines starting with # are ignored.\n"
    )
}

pub fn ensure(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    write(path, &list_header(path))
}

pub fn is_exe(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".exe")
}

pub fn default_label(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base)
        .to_owned()
}

pub fn add(path: &Path, label: &str, exe: &str) -> Result<(), String> {
    if label.trim().is_empty() {
        return Err("enter a name".to_owned());
    }
    if !is_exe(exe) {
        return Err("only .exe files can be added".to_owned());
    }
    ensure(path)?;
    let mut text = text(path);
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("{}|{}\n", label.trim(), exe.trim()));
    write(path, &text)
}

pub fn remove(path: &Path, exe: &str) -> Result<bool, String> {
    let text = text(path);
    let mut out = String::with_capacity(text.len());
    let mut removed = false;
    for line in text.split('\n') {
        let probe = line.trim_end_matches('\r').trim_start();
        if !probe.is_empty() && !probe.starts_with('#') {
            if let Some((_, path)) = probe.split_once('|') {
                if expand_home(path.trim()) == exe {
                    removed = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if removed {
        write(path, out.trim_end_matches('\n'))?;
    }
    Ok(removed)
}

pub fn rename(path: &Path, old_label: &str, new_label: &str) -> Result<bool, String> {
    let text = text(path);
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    for line in text.split('\n') {
        let probe = line.trim_end_matches('\r').trim_start();
        if !probe.is_empty() && !probe.starts_with('#') {
            if let Some((label, value)) = probe.split_once('|') {
                if label.trim() == old_label {
                    out.push_str(&format!("{}|{}\n", new_label.trim(), value.trim()));
                    changed = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if changed {
        write(path, out.trim_end_matches('\n'))?;
    }
    Ok(changed)
}

pub fn relink(path: &Path, old_path: &str, new_path: &str) -> Result<bool, String> {
    if !is_exe(new_path) {
        return Err("only .exe files can be linked".to_owned());
    }
    let text = text(path);
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    for line in text.split('\n') {
        let probe = line.trim_end_matches('\r').trim_start();
        if !probe.is_empty() && !probe.starts_with('#') {
            if let Some((label, value)) = probe.split_once('|') {
                if expand_home(value.trim()) == old_path {
                    out.push_str(&format!("{}|{}\n", label.trim(), new_path.trim()));
                    changed = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if changed {
        write(path, out.trim_end_matches('\n'))?;
    }
    Ok(changed)
}

/// Backup of a per-game file that is currently disabled (`<file>.off`).
pub fn off_path(own: &Path) -> PathBuf {
    PathBuf::from(format!("{}.off", own.display()))
}

/// Per-game file state: shared / game / disabled.
pub fn scope(shared: &Path, which: &str, appid: &str) -> &'static str {
    let Some(own) = crate::paths::Paths::game_file_named(shared, which, appid) else {
        return "shared";
    };
    if own.exists() {
        return "game";
    }
    if off_path(&own).exists() {
        "disabled"
    } else {
        "shared"
    }
}

/// `list_scope`: switch between the shared list and the per-game one.
///
/// `game` promotes the per-game file: a previously disabled one is restored,
/// and a fresh one is a copy of the shared list (`copy`) or an empty list.
/// `shared` keeps the per-game file as `<file>.off`, so "This game only" can
/// restore it; the effective list is the shared one again.
pub fn scope_action(shared: &Path, own: &Path, action: &str, copy: bool) -> Result<(), String> {
    match action {
        "game" => {
            let off = off_path(own);
            if off.exists() {
                std::fs::rename(&off, own).map_err(|e| e.to_string())?;
            } else if !own.exists() {
                if copy && shared.is_file() {
                    std::fs::copy(shared, own).map_err(|e| e.to_string())?;
                } else {
                    ensure(own)?;
                }
            }
        }
        "shared" => {
            if own.exists() {
                std::fs::rename(own, off_path(own)).map_err(|e| e.to_string())?;
            }
        }
        "delete" => {
            let _ = std::fs::remove_file(own);
            let _ = std::fs::remove_file(off_path(own));
        }
        other => return Err(format!("unknown scope action: {other}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mp-{name}-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn parses_comments_and_tildes() {
        let entries = parse_entries("# x\n\nA|/games/a.exe\r\nB|~/b.exe\nbad line\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "A");
        assert!(entries[1].path.ends_with("/b.exe"));
    }

    #[test]
    fn add_remove_rename_relink() {
        let dir = temp("lists");
        let list = dir.join("launchers.list");
        add(&list, "TruckersMP", "/games/launcher.exe").unwrap();
        add(&list, "TrucksBook", "/games/tb.exe").unwrap();
        assert_eq!(read_entries(&list).len(), 2);

        assert!(rename(&list, "TrucksBook", "TB").unwrap());
        assert_eq!(read_entries(&list)[1].label, "TB");
        assert!(relink(&list, "/games/launcher.exe", "/new/launcher.exe").unwrap());
        assert_eq!(read_entries(&list)[0].path, "/new/launcher.exe");
        assert!(remove(&list, "/games/tb.exe").unwrap());
        assert_eq!(read_entries(&list).len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_bad_exe() {
        let dir = temp("lists-bad");
        let list = dir.join("extras.list");
        assert!(add(&list, "x", "/tmp/not-exe").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scope_game_copies_the_shared_list() {
        let dir = temp("lists-scope");
        let shared = dir.join("launchers.list");
        let own = dir.join("launchers-227300.list");
        add(&shared, "TruckersMP", "/games/launcher.exe").unwrap();

        // What the manager sends: promote with a copy, so the entries survive.
        scope_action(&shared, &own, "game", true).unwrap();
        assert_eq!(read_entries(&own).len(), 1);
        assert_eq!(read_entries(&own)[0].label, "TruckersMP");

        // "Shared by all games" keeps the per-game file as a backup ...
        scope_action(&shared, &own, "shared", false).unwrap();
        assert!(!own.exists());
        assert!(off_path(&own).exists());

        // ... and promoting again restores it instead of emptying it.
        scope_action(&shared, &own, "game", true).unwrap();
        assert_eq!(read_entries(&own).len(), 1);
        assert!(!off_path(&own).exists());

        // Without the copy flag a fresh per-game list starts empty (API).
        let other = dir.join("launchers-270880.list");
        scope_action(&shared, &other, "game", false).unwrap();
        assert!(read_entries(&other).is_empty());

        scope_action(&shared, &other, "delete", false).unwrap();
        assert!(!other.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
