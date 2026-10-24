//! Run logs under `~/.local/state/steam-mp-launcher/logs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local};
use regex::Regex;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Run {
    pub path: String,
    pub name: String,
    pub mtime: u64,
    pub appid: String,
    pub game: String,
    pub launcher: String,
    pub status: String,
}

fn search(pattern: &str, text: &str) -> String {
    Regex::new(pattern)
        .ok()
        .and_then(|re| re.captures(text).map(|c| c[1].trim().to_owned()))
        .unwrap_or_default()
}

pub fn parse_run(path: &Path, mtime: SystemTime) -> Run {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let appid = Regex::new(r"-(\d+)\.log$")
        .ok()
        .and_then(|re| re.captures(&name).map(|c| c[1].to_owned()))
        .unwrap_or_default();
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut game = search(r"(?m)^.*\]\s+game\s*:\s*(.+?)\s*$", &text);
    let mut launcher = search(
        r"(?m)^.*\]\s+launcher\s*(?:\([^)]+\))?\s*:\s*(.+?)\s*$",
        &text,
    );
    if launcher.starts_with("none") || launcher.starts_with("<none>") {
        launcher.clear();
    } else if !launcher.is_empty() {
        let base = launcher.rsplit('/').next().unwrap_or(&launcher).to_owned();
        launcher = base.strip_suffix(".exe").unwrap_or(&base).to_owned();
    }
    if game.is_empty() {
        game = search(r"(?m)^.*\]\s+game\s+exe\s*:\s*(.+?)\s*$", &text);
    }
    Run {
        path: path.to_string_lossy().into_owned(),
        name,
        mtime: mtime
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        appid,
        game,
        launcher,
        status: if text.contains("ERROR:") || text.contains("ERROR :") {
            "error"
        } else {
            "ok"
        }
        .to_owned(),
    }
}

pub fn list_runs(dir: &Path, limit: usize) -> Vec<Run> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_file() {
            out.push(parse_run(
                &path,
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ));
        }
    }
    out.sort_by_key(|run| std::cmp::Reverse(run.mtime));
    if limit > 0 {
        out.truncate(limit);
    }
    out
}

pub fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

pub fn time_label(mtime: u64) -> String {
    let Some(stamp) = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(mtime)) else {
        return String::new();
    };
    let when: DateTime<Local> = stamp.into();
    let now = Local::now();
    if when.date_naive() == now.date_naive() {
        format!("today {}", when.format("%H:%M"))
    } else if when.date_naive() == now.date_naive() - chrono::Days::new(1) {
        format!("yesterday {}", when.format("%H:%M"))
    } else {
        when.format("%d.%m %H:%M").to_string()
    }
}

pub fn clear_older_than(dir: &Path, days: u64) -> Vec<String> {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(days.saturating_mul(86400)))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut removed = Vec::new();
    for run in list_runs(dir, 0) {
        let path = PathBuf::from(&run.path);
        if let Ok(meta) = fs::metadata(&path) {
            if meta.modified().unwrap_or(SystemTime::UNIX_EPOCH) < cutoff
                && fs::remove_file(&path).is_ok()
            {
                removed.push(run.name);
            }
        }
    }
    removed
}

/// Validate that a log path belongs to the logs directory (read_log command).
pub fn is_in_dir(dir: &Path, path: &Path) -> bool {
    let Ok(dir) = fs::canonicalize(dir) else {
        return false;
    };
    let Ok(path) = fs::canonicalize(path) else {
        return false;
    };
    path.starts_with(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mp-{name}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn error_status_accepts_both_spellings() {
        let dir = scratch("logs-status");
        let path = dir.join("mp-Game-227300.log");
        fs::write(&path, "[t] game : Game\n[t] ERROR: boom\n").unwrap();
        assert_eq!(parse_run(&path, SystemTime::now()).status, "error");
        // the key/value logger used to write "ERROR : …"
        fs::write(&path, "[t] game : Game\n[t] ERROR : boom\n").unwrap();
        assert_eq!(parse_run(&path, SystemTime::now()).status, "error");
        fs::write(&path, "[t] game : Game\n[t] === done (rc=0) ===\n").unwrap();
        assert_eq!(parse_run(&path, SystemTime::now()).status, "ok");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn parses_the_canonical_header_lines() {
        let dir = scratch("logs-header");
        let path = dir.join("mp-Euro_Truck_Simulator_2-227300.log");
        fs::write(
            &path,
            "[t] game              : Euro Truck Simulator 2\n\
             [t] launcher (chosen) : /games/TruckersMP-Launcher.exe\n\
             [t] done : rc=0\n",
        )
        .unwrap();
        let run = parse_run(&path, SystemTime::now());
        assert_eq!(run.appid, "227300");
        assert_eq!(run.game, "Euro Truck Simulator 2");
        assert_eq!(run.launcher, "TruckersMP-Launcher");
        fs::remove_dir_all(dir).ok();
    }
}
