//! Games that use the tool, from config.vdf and per-game files.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Serialize;

use crate::protons::steam_libraries;

pub const TOOL_NAME: &str = "steam-mp-launcher";

#[derive(Clone, Debug, Serialize)]
pub struct Game {
    pub appid: String,
    pub name: String,
    pub prefix: String,
    pub source: String,
    pub exe: String,
    pub compatdata: String,
}

pub fn vdf_field(path: &Path, key: &str) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let pattern = format!(r#"(?m)^[ \t]*"{key}"[ \t]*"(.*)"[ \t]*$"#);
    Regex::new(&pattern)
        .ok()
        .and_then(|re| re.captures(&text).map(|c| c[1].to_owned()))
        .unwrap_or_default()
}

pub fn appmanifest_for(appid: &str, root: &Path) -> Option<PathBuf> {
    if !appid.bytes().all(|b| b.is_ascii_digit()) || appid == "0" {
        return None;
    }
    for lib in steam_libraries(root) {
        let manifest = lib.join(format!("steamapps/appmanifest_{appid}.acf"));
        if manifest.is_file() {
            return Some(manifest);
        }
    }
    None
}

pub fn compat_appids(root: &Path) -> Vec<String> {
    let text = fs::read_to_string(root.join("config/config.vdf")).unwrap_or_default();
    let block = Regex::new(r#""(\d+)"\s*\{([^{}]*)\}"#).unwrap();
    let name = Regex::new(r#""name"\s+"([^"]+)""#).unwrap();
    let mut out = Vec::new();
    for capture in block.captures_iter(&text) {
        if name
            .captures(&capture[2])
            .is_some_and(|n| n[1].starts_with(TOOL_NAME))
        {
            out.push(capture[1].to_owned());
        }
    }
    out
}

pub fn per_game_appids(conf_dir: &Path) -> Vec<String> {
    let mut ids = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(conf_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rest = if let Some(rest) = name.strip_prefix("launcher-") {
                Some(rest)
            } else if let Some(rest) = name.strip_prefix("launchers-") {
                Some(rest.strip_suffix(".list").unwrap_or(rest))
            } else {
                name.strip_prefix("extras-")
                    .map(|r| r.strip_suffix(".list").unwrap_or(r))
            };
            if let Some(id) = rest {
                if !id.is_empty() && id != "0" && id.bytes().all(|b| b.is_ascii_digit()) {
                    ids.insert(id.to_owned());
                }
            }
        }
    }
    ids.into_iter().collect()
}

pub fn prefix_for(root: &Path, appid: &str) -> Option<PathBuf> {
    if !appid.bytes().all(|b| b.is_ascii_digit()) || appid == "0" {
        return None;
    }
    for lib in steam_libraries(root) {
        let compatdata = lib.join("steamapps/compatdata").join(appid);
        if compatdata.is_dir() {
            return Some(compatdata);
        }
    }
    None
}

pub fn game_dir(root: &Path, appid: &str) -> Option<PathBuf> {
    let manifest = appmanifest_for(appid, root)?;
    let installdir = vdf_field(&manifest, "installdir");
    if installdir.is_empty() {
        return None;
    }
    let lib = manifest.parent()?.parent()?;
    let dir = lib.join("steamapps/common").join(installdir);
    dir.is_dir().then_some(dir)
}

pub fn find_exe(root: &Path, appid: &str) -> String {
    let Some(dir) = game_dir(root, appid) else {
        return String::new();
    };
    let candidates = [
        "bin/win_x64/eurotrucks2.exe",
        "bin/win_x64/amtrucks.exe",
        "bin/x64/eurotrucks2.exe",
        "bin/x64/amtrucks.exe",
        "eurotrucks2.exe",
        "amtrucks.exe",
    ];
    for rel in candidates {
        let path = dir.join(rel);
        if path.is_file() {
            return path.to_string_lossy().into_owned();
        }
    }
    // Last resort: walk two levels for the known executable names.
    let wanted = ["eurotrucks2.exe", "amtrucks.exe"];
    for base in [dir.join("bin"), dir.clone()] {
        if let Ok(entries) = fs::read_dir(base) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if wanted.contains(&name.as_str()) {
                    return entry.path().to_string_lossy().into_owned();
                }
            }
        }
    }
    String::new()
}

pub fn game_name(root: &Path, appid: &str) -> String {
    appmanifest_for(appid, root)
        .map(|path| vdf_field(&path, "name"))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("app{appid}"))
}

pub fn detect_games(root: &Path, conf_dir: &Path) -> Vec<Game> {
    let from_steam = compat_appids(root);
    let from_files = per_game_appids(conf_dir);
    let mut candidates: Vec<String> = from_steam.clone();
    candidates.extend(from_files.iter().cloned());
    for known in ["227300", "270880"] {
        if appmanifest_for(known, root).is_some() && !candidates.iter().any(|id| id == known) {
            candidates.push(known.to_owned());
        }
    }
    let mut out: Vec<Game> = Vec::new();
    let mut seen = BTreeSet::new();
    for appid in candidates {
        if !seen.insert(appid.clone()) {
            continue;
        }
        let compatdata = prefix_for(root, &appid);
        out.push(Game {
            appid: appid.clone(),
            name: game_name(root, &appid),
            prefix: compatdata
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            source: if from_steam.contains(&appid) {
                "steam"
            } else if from_files.contains(&appid) {
                "files"
            } else {
                "known"
            }
            .to_owned(),
            exe: find_exe(root, &appid),
            compatdata: compatdata
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        });
    }
    out.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    out
}

pub fn resolve_appid(requested: &str, root: Option<&Path>, conf_dir: &Path) -> String {
    if !requested.is_empty() {
        return requested.to_owned();
    }
    for name in ["SteamAppId", "SteamGameId"] {
        if let Ok(value) = std::env::var(name) {
            if !value.is_empty() && value != "0" && value.bytes().all(|b| b.is_ascii_digit()) {
                return value;
            }
        }
    }
    if let Some(root) = root {
        if appmanifest_for("227300", root).is_some() {
            return "227300".to_owned();
        }
        if let Some(game) = detect_games(root, conf_dir).into_iter().next() {
            return game.appid;
        }
    }
    "227300".to_owned()
}

pub fn games_using_tool(root: Option<&Path>, conf_dir: &Path) -> Vec<Game> {
    root.map(|root| detect_games(root, conf_dir))
        .unwrap_or_default()
}
