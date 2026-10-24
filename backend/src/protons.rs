//! Proton discovery, including distro builds under /usr/share/steam.

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use regex::Regex;
use serde::Serialize;

use crate::config::Config;
use crate::paths::home;

#[derive(Clone, Debug, Serialize)]
pub struct Proton {
    pub name: String,
    pub dir: String,
    pub source: String,
}

pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(root) = env::var("STEAM_COMPAT_CLIENT_INSTALL_PATH") {
        if !root.is_empty() {
            roots.push(PathBuf::from(root.trim_end_matches('/')));
        }
    }
    let home = home();
    for root in [
        home.join(".local/share/Steam"),
        home.join(".steam/root"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
    ] {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

pub fn steam_root() -> Option<PathBuf> {
    for root in steam_roots() {
        if root.join("steamapps").is_dir() {
            return Some(fs::canonicalize(&root).unwrap_or(root));
        }
    }
    None
}

pub fn steam_libraries(root: &Path) -> Vec<PathBuf> {
    let mut libs = Vec::new();
    if root.join("steamapps").is_dir() {
        libs.push(root.to_path_buf());
    }
    let text = fs::read_to_string(root.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
    let re = Regex::new(r#""path"\s+"([^"]+)""#).unwrap();
    for capture in re.captures_iter(&text) {
        let path = PathBuf::from(&capture[1]);
        if path.join("steamapps").is_dir() && !libs.contains(&path) {
            libs.push(path);
        }
    }
    libs
}

/// Distro compatibilitytools.d dirs; empty override disables the scan.
pub fn system_dirs() -> Vec<PathBuf> {
    let raw = env::var("MP_SYSTEM_COMPAT_DIRS").ok();
    let dirs: Vec<PathBuf> = match raw {
        Some(value) => value
            .split(':')
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect(),
        None => vec![
            PathBuf::from("/usr/share/steam/compatibilitytools.d"),
            PathBuf::from("/usr/local/share/steam/compatibilitytools.d"),
            PathBuf::from("/usr/lib/steam/compatibilitytools.d"),
        ],
    };
    dirs.into_iter().filter(|dir| dir.is_dir()).collect()
}

fn executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn children(base: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(base)
        .map(|it| it.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn tool_paths() -> Vec<PathBuf> {
    env::var("STEAM_COMPAT_TOOL_PATHS")
        .unwrap_or_default()
        .split(':')
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Every directory holding an executable `proton`.
pub fn each_proton_dir(root: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for dir in tool_paths() {
        if executable_file(&dir.join("proton")) {
            dirs.push(dir);
        }
    }
    for base in system_dirs() {
        for dir in children(&base) {
            if executable_file(&dir.join("proton")) {
                dirs.push(dir);
            }
        }
    }
    if let Some(root) = root {
        for lib in steam_libraries(root) {
            for parent in ["steamapps/common", "compatibilitytools.d"] {
                for dir in children(&lib.join(parent)) {
                    if executable_file(&dir.join("proton")) {
                        dirs.push(dir);
                    }
                }
            }
        }
    }
    dirs
}

pub fn list_protons(root: Option<&Path>) -> Vec<Proton> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for dir in each_proton_dir(root) {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Proton")
            .to_owned();
        if !seen.insert(name.clone()) {
            continue;
        }
        let source = if let Some(root) = root {
            if dir.starts_with(root.join("steamapps/common")) {
                "Steam"
            } else {
                "compatibilitytools.d"
            }
        } else {
            "system"
        };
        out.push(Proton {
            name,
            dir: dir.to_string_lossy().into_owned(),
            source: source.to_owned(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn score(name: &str) -> i32 {
    let low = name.to_ascii_lowercase();
    if low == "proton - experimental" {
        50
    } else if low.starts_with("proton ") && low.as_bytes().get(7).is_some_and(u8::is_ascii_digit) {
        40
    } else if low.starts_with("proton-cachyos") || low.starts_with("ge-proton") {
        30
    } else if low.starts_with("proton") {
        20
    } else {
        10
    }
}

fn mtime(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

pub fn pick_proton(root: Option<&Path>, prefix_conf: Option<&Path>) -> Option<PathBuf> {
    let prefix_owner = prefix_conf
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default();
    let mut best: Option<(i32, SystemTime, PathBuf)> = None;
    // Kept as a Vec so the mtime tie-break is deterministic and cheap.
    let mut dirs = each_proton_dir(root);
    dirs.sort_by_key(|d| std::cmp::Reverse(mtime(d)));
    for dir in dirs {
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let mut value = score(name);
        if !prefix_owner.is_empty()
            && prefix_owner
                .lines()
                .any(|line| line.starts_with(&dir.to_string_lossy().into_owned()))
        {
            value += 100;
        }
        let candidate = (value, mtime(&dir), dir);
        if best
            .as_ref()
            .is_none_or(|b| candidate.0 > b.0 || (candidate.0 == b.0 && candidate.1 > b.1))
        {
            best = Some(candidate);
        }
    }
    best.map(|(_, _, dir)| dir.join("proton"))
}

pub fn pick_by_name(name: &str, root: Option<&Path>) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    for dir in each_proton_dir(root) {
        let base = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if base == name {
            return Some(dir.join("proton"));
        }
        if let Ok(vdf) = fs::read_to_string(dir.join("compatibilitytool.vdf")) {
            if vdf.contains(&format!("\"{name}\"")) {
                return Some(dir.join("proton"));
            }
        }
    }
    None
}

pub fn resolve(
    config: &Config,
    root: Option<&Path>,
    prefix_conf: Option<&Path>,
) -> Option<PathBuf> {
    let wanted = config.get("PROTON", "");
    if !wanted.is_empty() {
        if wanted.starts_with('/') {
            let path = PathBuf::from(&wanted);
            if executable_file(&path) {
                return Some(path);
            }
            if executable_file(&path.join("proton")) {
                return Some(path.join("proton"));
            }
        } else if executable_file(&PathBuf::from(&wanted).join("proton")) {
            return Some(PathBuf::from(&wanted).join("proton"));
        } else if let Some(path) = pick_by_name(&wanted, root) {
            return Some(path);
        }
    }
    pick_proton(root, prefix_conf)
}

pub fn prefix_of(compatdata: &Path) -> PathBuf {
    let pfx = compatdata.join("pfx");
    if pfx.join("drive_c").is_dir() {
        pfx
    } else if compatdata.join("drive_c").is_dir() {
        compatdata.to_path_buf()
    } else {
        pfx
    }
}

pub fn proton_name(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("Proton")
        .to_owned()
}
