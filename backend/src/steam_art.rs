//! Steam library art: a square icon and a wide banner from the local cache.

use std::fs;
use std::path::{Path, PathBuf};

use crate::protons::steam_roots;

fn art_files(folder: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(folder) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Ok(inner) = fs::read_dir(&path) {
                for file in inner.flatten() {
                    let name = file.file_name().to_string_lossy().to_ascii_lowercase();
                    if name.ends_with(".jpg") || name.ends_with(".png") {
                        out.push((name, file.path()));
                    }
                }
            }
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        if name.ends_with(".jpg") || name.ends_with(".png") {
            out.push((name, path));
        }
    }
    out
}

fn cache(appid: &str) -> Vec<(String, PathBuf)> {
    if !appid.bytes().all(|b| b.is_ascii_digit()) || appid == "0" {
        return Vec::new();
    }
    for root in steam_roots() {
        let folder = root.join("appcache/librarycache").join(appid);
        let files = art_files(&folder);
        if !files.is_empty() {
            return files;
        }
    }
    Vec::new()
}

pub fn find_icon(appid: &str) -> String {
    let files = cache(appid);
    // Steam keeps the square icon as a 40-char hash; prefer it over logo.png.
    for (name, path) in &files {
        let stem = name.split('.').next().unwrap_or("");
        if stem.len() == 40 && stem.bytes().all(|b| b.is_ascii_hexdigit()) {
            return path.to_string_lossy().into_owned();
        }
    }
    for wanted in ["icon", "header"] {
        for (name, path) in &files {
            if name.contains(wanted) && !name.contains("blur") {
                return path.to_string_lossy().into_owned();
            }
        }
    }
    for (name, path) in &files {
        if !name.contains("logo") && !name.contains("blur") {
            return path.to_string_lossy().into_owned();
        }
    }
    files
        .first()
        .map(|(_, path)| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn find_banner(appid: &str) -> String {
    let files = cache(appid);
    for wanted in [
        "library_hero.jpg",
        "library_hero.png",
        "library_header.jpg",
        "library_header.png",
        "header.jpg",
        "header.png",
        "library_capsule.jpg",
        "library_capsule.png",
        "library_600x900.jpg",
        "library_600x900.png",
    ] {
        for (name, path) in &files {
            if name == wanted {
                return path.to_string_lossy().into_owned();
            }
        }
    }
    files
        .iter()
        .filter(|(name, _)| !name.contains("logo") && !name.contains("blur"))
        .max_by_key(|(_, path)| fs::metadata(path).map(|m| m.len()).unwrap_or(0))
        .map(|(_, path)| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}
