//! Guided installers for TruckersMP and TrucksBook.
//!
//! Port of `mpgui/core/installers.py`: find the download, unpack a ZIP safely,
//! run the setup through Proton, wait for the fresh TruckersMP launcher to
//! finish verifying files, close it, point the game at the real executable and
//! add the launcher/sidecar to the right list.

use std::cmp::Reverse;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde_json::{json, Value};

use crate::games;
use crate::lists;
use crate::paths::{atomic_write, Paths};
use crate::protons;

/// Exactly the marker the Python installer knows.  TruckersMP 1.3.x does not
/// write it, so the launcher is left open and the user closes it (then clicks
/// "I closed it"); killing it automatically made the window vanish.
const READY_MARKERS: [&str; 1] = ["Verify files operation finished"];
const TRUCKERSMP_LABEL: &str = "TruckersMP";
const TRUCKSBOOK_LABEL: &str = "TrucksBook";

const TRUCKSBOOK_APPIDS: [&str; 2] = ["227300", "270880"];
const TRUCKERSMP_GAME_EXE: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Euro Truck Simulator 2\bin\win_x64\eurotrucks2.exe";

/// A complete `app-<ver>/TruckersMP-Launcher.exe` is ~211 MB; the execution
/// stub in the install root is only ~396 kB, so the size separates a finished
/// unpack from one in progress (1.3.19: 211 237 432 B, stub 395 968 B).
const LAUNCHER_FULL_BYTES: u64 = 200_000_000;

// -- downloads ------------------------------------------------------------

fn expand_home(value: &str) -> String {
    let home = crate::paths::home();
    value
        .replace("${HOME}", &home.to_string_lossy())
        .replace("$HOME", &home.to_string_lossy())
        .replace("~/", &format!("{}/", home.to_string_lossy()))
        .replace("~", &home.to_string_lossy())
}

/// `XDG_DOWNLOAD_DIR`, then `user-dirs.dirs`, then Downloads/Pobrane.
pub fn downloads_dir() -> PathBuf {
    if let Ok(value) = env::var("XDG_DOWNLOAD_DIR") {
        if !value.is_empty() {
            let path = PathBuf::from(expand_home(&value));
            if path.is_dir() {
                return path;
            }
        }
    }
    let home = crate::paths::home();
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = fs::read_to_string(config.join("user-dirs.dirs")) {
        for line in text.lines() {
            let Some(value) = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=") else {
                continue;
            };
            let value = value.trim().trim_matches('"');
            let path = PathBuf::from(expand_home(value));
            if path.is_dir() {
                return path;
            }
        }
    }
    for name in ["Downloads", "Pobrane", "Pobierane"] {
        let path = home.join(name);
        if path.is_dir() {
            return path;
        }
    }
    home.join("Downloads")
}

/// Newest TruckersMP `.exe`/`.zip` in the download folder.
pub fn find_downloaded_installer(folder: Option<&Path>) -> String {
    let folder = folder.map(Path::to_path_buf).unwrap_or_else(downloads_dir);
    let Ok(entries) = fs::read_dir(&folder) else {
        return String::new();
    };
    let mut matches: Vec<(SystemTime, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let low = name.to_ascii_lowercase();
        if !(low.ends_with(".exe") || low.ends_with(".zip")) || !low.contains("truckersmp") {
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        matches.push((modified, path.to_string_lossy().into_owned()));
    }
    matches.sort_by_key(|entry| Reverse(entry.0));
    matches
        .first()
        .map(|(_, path)| path.clone())
        .unwrap_or_default()
}

fn contains_utf16le(haystack: &[u8], needle: &str) -> bool {
    let pattern: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
    !pattern.is_empty()
        && haystack
            .windows(pattern.len())
            .any(|window| window == pattern)
}

/// The TrucksBook setup in Downloads, detected by its PE version strings.
pub fn find_trucksbook_installer(folder: Option<&Path>) -> String {
    let folder = folder.map(Path::to_path_buf).unwrap_or_else(downloads_dir);
    let Ok(entries) = fs::read_dir(&folder) else {
        return String::new();
    };
    let mut candidates: Vec<(i32, SystemTime, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let low = name.to_ascii_lowercase();
        if !low.ends_with(".exe") || !path.is_file() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.len() > 300 * 1024 * 1024 {
            continue;
        }
        let mut score = 0;
        if low.contains("trucksbook") {
            score += 3;
        }
        if low.contains("setup") {
            score += 2;
        }
        if low.starts_with("tb") {
            score += 1;
        }
        candidates.push((
            score,
            metadata.modified().unwrap_or(UNIX_EPOCH),
            path.to_string_lossy().into_owned(),
        ));
    }
    candidates.sort_by_key(|entry| (Reverse(entry.0), Reverse(entry.1)));
    for (_, _, path) in candidates {
        let Ok(mut file) = fs::File::open(&path) else {
            continue;
        };
        let mut head = vec![0u8; 8 * 1024 * 1024];
        let read = file.read(&mut head).unwrap_or(0);
        head.truncate(read);
        if contains_utf16le(&head, "TrucksBook")
            || head.windows(17).any(|w| w == b"TrucksBook Client")
        {
            return path;
        }
    }
    String::new()
}

// -- zip helpers ----------------------------------------------------------

fn looks_like_pe(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 2];
    file.read_exact(&mut magic).is_ok() && &magic == b"MZ"
}

fn zip_names(zip: &Path) -> Result<Vec<String>, String> {
    if let Ok(output) = Command::new("unzip").args(["-Z1"]).arg(zip).output() {
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect());
        }
    }
    if let Ok(output) = Command::new("bsdtar").args(["-tf"]).arg(zip).output() {
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect());
        }
    }
    Err("not a zip archive".to_owned())
}

fn safe_name(name: &str) -> Result<(), String> {
    let normal = name.replace('\\', "/");
    if normal.starts_with('/') || normal.contains(':') {
        return Err(format!("unsafe path in ZIP: {name}"));
    }
    let mut depth = 0i32;
    for part in normal.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!("unsafe path in ZIP: {name}"));
                }
            }
            _ => depth += 1,
        }
    }
    Ok(())
}

fn extract_zip(zip: &Path, dest: &Path) -> Result<(), String> {
    let unzip = Command::new("unzip")
        .args(["-o", "-q"])
        .arg(zip)
        .arg("-d")
        .arg(dest)
        .status();
    if unzip.is_ok_and(|status| status.success()) {
        return Ok(());
    }
    let tar = Command::new("bsdtar")
        .arg("-xf")
        .arg(zip)
        .arg("-C")
        .arg(dest)
        .status();
    if tar.is_ok_and(|status| status.success()) {
        return Ok(());
    }
    let seven = Command::new("7z")
        .arg("x")
        .arg("-y")
        .arg(format!("-o{}", dest.display()))
        .arg(zip)
        .status();
    if seven.is_ok_and(|status| status.success()) {
        return Ok(());
    }
    Err("could not unpack the archive".to_owned())
}

fn installer_exe(folder: &Path) -> String {
    fn walk(base: &Path, root: &Path, best: &mut Option<((i32, i32, u64), String)>) {
        let Ok(entries) = fs::read_dir(base) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, best);
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            if !name.ends_with(".exe") {
                continue;
            }
            let mut score = 0;
            if name.contains("setup") {
                score += 8;
            }
            if name.contains("install") {
                score += 6;
            }
            if name.contains("truckersmp") {
                score += 4;
            }
            if name.contains("launcher") {
                score += 2;
            }
            let depth = base
                .strip_prefix(root)
                .map(|rest| rest.components().count() as i32)
                .unwrap_or(0);
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let key = (score, -depth, size);
            if best.as_ref().is_none_or(|(best_key, _)| key > *best_key) {
                *best = Some((key, path.to_string_lossy().into_owned()));
            }
        }
    }
    let mut best = None;
    walk(folder, folder, &mut best);
    best.map(|(_, path)| path).unwrap_or_default()
}

fn temp_dir() -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = env::temp_dir().join(format!("mp-truckersmp-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// A leftover `mp-truckersmp-*` tree is an unpacked installer (up to ~200 MB);
/// a crash or a caller that never cleans up must not fill `/tmp` forever.
fn is_stale_temp(name: &str, age: Duration) -> bool {
    name.starts_with("mp-truckersmp-") && age > Duration::from_secs(6 * 3600)
}

fn clean_stale_temp_dirs() {
    let Ok(entries) = fs::read_dir(env::temp_dir()) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::getuid() } {
            continue;
        }
        let age = meta
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .unwrap_or_default();
        if is_stale_temp(&name, age) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// The setup executable for a chosen `.exe` or `.zip`.
pub fn resolve_installer(path: &str) -> Result<(String, String), String> {
    let source = Path::new(path);
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        match zip_names(source) {
            Ok(names) => {
                for name in &names {
                    safe_name(name)?;
                }
                let dir = temp_dir()?;
                if let Err(error) = extract_zip(source, &dir) {
                    let _ = fs::remove_dir_all(&dir);
                    return Err(error);
                }
                return Ok((installer_exe(&dir), dir.to_string_lossy().into_owned()));
            }
            Err(error) => {
                if looks_like_pe(source) {
                    return Ok((path.to_owned(), String::new()));
                }
                return Err(error);
            }
        }
    }
    if lower.ends_with(".exe") || looks_like_pe(source) {
        return Ok((path.to_owned(), String::new()));
    }
    Ok((String::new(), String::new()))
}

// -- Steam prefix helpers -------------------------------------------------

fn appdata_dirs(prefix: &Path) -> Vec<PathBuf> {
    appdata_dirs_in(prefix, &["Local", "Roaming"])
}

fn appdata_dirs_in(prefix: &Path, subs: &[&str]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Ok(users) = fs::read_dir(prefix.join("drive_c/users")) else {
        return dirs;
    };
    for user in users.flatten() {
        for sub in subs {
            let dir = user.path().join("AppData").join(sub);
            if dir.is_dir() {
                dirs.push(dir);
            }
        }
    }
    dirs
}

fn walk_exe(base: &Path, wanted: &str, fallback_word: &str) -> String {
    let mut fallback = String::new();
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            if !name.ends_with(".exe") {
                continue;
            }
            if name == wanted {
                return path.to_string_lossy().into_owned();
            }
            if fallback.is_empty() && name.contains(fallback_word) {
                fallback = path.to_string_lossy().into_owned();
            }
        }
    }
    fallback
}

fn launcher_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            if name.ends_with(".exe") && name.contains("truckersmp") && name.contains("launcher") {
                found.push(path);
            }
        }
    }
    found
}

/// The installed TruckersMP launcher: the exact name wins, then the newest
/// other `*truckersmp*launcher*.exe` (an update leaves the old version around
/// for a while; the Python installer ranked the same way).
pub fn find_truckersmp_launcher(prefix: &Path) -> String {
    if prefix.as_os_str().is_empty() {
        return String::new();
    }
    for dir in appdata_dirs(prefix) {
        let exact = dir.join("TruckersMP/TruckersMP-Launcher.exe");
        if exact.is_file() {
            return exact.to_string_lossy().into_owned();
        }
    }
    let mut roots = appdata_dirs(prefix);
    let programdata = prefix.join("drive_c/ProgramData/TruckersMP");
    if programdata.is_dir() {
        roots.push(programdata);
    }
    let mut best: Option<(bool, SystemTime, PathBuf)> = None;
    for root in roots {
        for path in launcher_files(&root) {
            let exact = path
                .file_name()
                .map(|name| name.to_string_lossy().eq_ignore_ascii_case("TruckersMP-Launcher.exe"))
                .unwrap_or(false);
            let modified = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let better = match &best {
                None => true,
                Some((best_exact, best_modified, _)) => {
                    (exact && !best_exact) || (exact == *best_exact && modified > *best_modified)
                }
            };
            if better {
                best = Some((exact, modified, path));
            }
        }
    }
    best.map(|(_, _, path)| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The installed `TB Client.exe` in the prefix.
pub fn find_trucksbook_client(prefix: &Path) -> String {
    let exact = prefix.join("drive_c/Program Files (x86)/TrucksBook Client/TB Client.exe");
    if exact.is_file() {
        return exact.to_string_lossy().into_owned();
    }
    for folder in ["Program Files (x86)", "Program Files"] {
        let root = prefix.join("drive_c").join(folder);
        if root.is_dir() {
            let found = walk_exe(&root, "tb client.exe", "trucksbook");
            if !found.is_empty() {
                return found;
            }
        }
    }
    String::new()
}

/// The ETS2/ATS executable to point the setup at.
pub fn find_trucksbook_game_exe(root: Option<&Path>, appid: &str) -> String {
    let Some(root) = root else {
        return String::new();
    };
    let order: Vec<&str> = if TRUCKSBOOK_APPIDS.contains(&appid) {
        vec![appid]
    } else {
        TRUCKSBOOK_APPIDS.to_vec()
    };
    for candidate in order {
        let path = games::find_exe(root, candidate);
        if !path.is_empty() {
            return path;
        }
    }
    String::new()
}

fn find_options(prefix: &Path) -> Option<PathBuf> {
    // Roaming first, like the Python installer (the launcher writes there).
    appdata_dirs_in(prefix, &["Roaming", "Local"])
        .into_iter()
        .map(|dir| dir.join("TruckersMP/launcher-options.json"))
        .find(|path| path.is_file())
}

/// `S:\...` for a library under the Steam root, `Z:\home/...` otherwise.
pub fn windows_game_path(game_exe: &Path, root: Option<&Path>) -> String {
    if game_exe.as_os_str().is_empty() {
        return String::new();
    }
    let absolute = fs::canonicalize(game_exe).unwrap_or_else(|_| game_exe.to_path_buf());
    if let Some(root) = root {
        let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        if let Ok(relative) = absolute.strip_prefix(&root) {
            return format!("S:\\{}", relative.to_string_lossy().replace('/', "\\"));
        }
    }
    format!(
        "Z:\\{}",
        absolute
            .to_string_lossy()
            .trim_start_matches('/')
            .replace('/', "\\")
    )
}

pub fn set_truckersmp_game_path(prefix: &Path, game_path: &str) -> (bool, String) {
    let Some(config) = find_options(prefix) else {
        return (false, String::new());
    };
    // The default value goes through Proton's S: drive.  A prefix without S:
    // would only get a JSON file that points at nothing.
    if game_path == TRUCKERSMP_GAME_EXE && !prefix.join("dosdevices/s:").exists() {
        return (false, config.to_string_lossy().into_owned());
    }
    let text = fs::read_to_string(&config).unwrap_or_default();
    let Ok(mut data) = serde_json::from_str::<Value>(&text) else {
        return (false, config.to_string_lossy().into_owned());
    };
    if !data.is_object() {
        return (false, config.to_string_lossy().into_owned());
    }
    let games = data
        .as_object_mut()
        .unwrap()
        .entry("games")
        .or_insert_with(|| json!({}));
    if !games.is_object() {
        *games = json!({});
    }
    let ets2 = games
        .as_object_mut()
        .unwrap()
        .entry("ets2")
        .or_insert_with(|| json!({"game": "ets2", "consoleOpts": []}));
    if !ets2.is_object() {
        *ets2 = json!({"game": "ets2", "consoleOpts": []});
    }
    ets2.as_object_mut()
        .unwrap()
        .insert("path".to_owned(), json!(game_path));
    let result = serde_json::to_string(&data).unwrap_or_default();
    match atomic_write(&config, &result) {
        Ok(()) => (true, config.to_string_lossy().into_owned()),
        Err(_) => (false, config.to_string_lossy().into_owned()),
    }
}

// -- install state --------------------------------------------------------

/// `app-1.3.19` -> `[1, 3, 19]` (older/odd names sort last).
fn version_key(name: &str) -> Option<Vec<u64>> {
    let raw = name.strip_prefix("app-")?;
    let mut parts = Vec::new();
    for part in raw.split('.') {
        parts.push(part.parse::<u64>().ok()?);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts)
}

fn truckersmp_roots(prefix: &Path) -> Vec<PathBuf> {
    appdata_dirs(prefix)
        .into_iter()
        .map(|dir| dir.join("TruckersMP"))
        .filter(|dir| dir.is_dir())
        .collect()
}

/// The highest `app-<ver>` directory under a TruckersMP install root.  An
/// update creates `app-<new>` before `cleanDeadVersions` removes the old one,
/// so "any app-*" would point at the previous version for a while.
fn newest_app(root: &Path) -> Option<(Vec<u64>, PathBuf)> {
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for entry in fs::read_dir(root).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(key) = version_key(&name) else {
            continue;
        };
        if !entry.path().is_dir() {
            continue;
        }
        if best.as_ref().is_none_or(|(best, _)| key > *best) {
            best = Some((key, entry.path()));
        }
    }
    best
}

/// Squirrel's own end marker: the staged updater appends this line right
/// before it removes the payload from `SquirrelTemp` (measured: +22.7 s of a
/// 22.6 s `--silent` install, i.e. together with `app.ico`).
fn squirrel_finished(prefix: &Path) -> bool {
    appdata_dirs(prefix).into_iter().any(|dir| {
        let log = dir.join("SquirrelTemp/Squirrel-Install.log");
        let Ok(text) = fs::read_to_string(&log) else {
            return false;
        };
        text.lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .is_some_and(|line| line.contains("Finished Squirrel Updater"))
    })
}

/// Is a Squirrel `Update.exe`/`Setup.exe` of *this* prefix still running?
/// Their command lines are identical in every prefix (`C:\users\steamuser\…`),
/// so only the environment (WINEPREFIX / STEAM_COMPAT_DATA_PATH) identifies
/// them.  Matching the whole command line is intentional: the `proton` wrapper
/// of the running installer also mentions `…\TruckersMP-Setup.exe`, and while
/// that tree is alive the install is still in progress.
fn squirrel_busy(prefix: &Path) -> bool {
    let needle = prefix.to_string_lossy();
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|pid| pid.parse::<i32>().ok())
        else {
            continue;
        };
        let Some(cmd) = process_cmdline(pid) else {
            continue;
        };
        let low = cmd.to_ascii_lowercase();
        if !(low.contains("update.exe") || low.contains("setup.exe")) {
            continue;
        }
        if fs::read(format!("/proc/{pid}/environ"))
            .is_ok_and(|env| String::from_utf8_lossy(&env).contains(needle.as_ref()))
        {
            return true;
        }
    }
    false
}

/// `(complete, installing, version)` for the Squirrel tree in `prefix`.
///
/// Measured with `TruckersMP-Setup.exe` 1.3.19 (cachyos-11.0, tmpfs prefix):
/// * `app-<ver>/TruckersMP-Launcher.exe` > 200 MB at +5.3 s,
/// * stub + `packages/RELEASES` at +5.3…5.9 s,
/// * `launcher-options.json` rewritten at +7.2 s,
/// * **no file changes at all from +8 s to +22 s** (Squirrel hook timeout),
/// * `app.ico` + `Finished Squirrel Updater` + payload removal at +22.7 s,
///   i.e. in the same sample as `Setup.exe`/`Update.exe` exiting.
///
/// So "big exe + stub + RELEASES" is true ~17 s early, and "no changes for
/// 3 s" fires ~11 s early as well; neither is a completion marker on its own.
fn install_state(prefix: &Path) -> (bool, bool, String) {
    let busy = squirrel_busy(prefix);
    let finished = squirrel_finished(prefix);
    // Before the first `app-<ver>` exists Squirrel still has its staged log in
    // `SquirrelTemp`; that is "installing", not "nothing here".
    let staged = appdata_dirs(prefix)
        .into_iter()
        .any(|dir| dir.join("SquirrelTemp/Squirrel-Install.log").is_file());
    let Some((root, (key, app))) = truckersmp_roots(prefix)
        .into_iter()
        .filter_map(|root| newest_app(&root).map(|app| (root, app)))
        .max_by(|left, right| left.1 .0.cmp(&right.1 .0))
    else {
        return (false, busy || (staged && !finished), String::new());
    };
    let version = key.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
    let size = fs::metadata(app.join("TruckersMP-Launcher.exe"))
        .map(|meta| meta.len())
        .unwrap_or(0);
    let stub = root.join("TruckersMP-Launcher.exe").is_file();
    let releases = root.join("packages/RELEASES").is_file();
    let icon = root.join("app.ico").is_file();
    // `app.ico` and the final log line are written by the staged updater *at
    // the very end*; without one of them a half-unpacked tree would pass the
    // size/stub/RELEASES test ~17 s early.
    let complete = size >= LAUNCHER_FULL_BYTES && stub && releases && !busy && (icon || finished);
    let installing = !complete
        && (busy || size < LAUNCHER_FULL_BYTES || !stub || !releases || !(icon || finished));
    (complete, installing, version)
}

/// Proton deletes `dosdevices/s:` whenever it is not given the library env
/// (cachyos-11.0, proton `setup_dir_drive`), and an older build of
/// `launch.rs` did not pass it.  `S:\...` in `launcher-options.json` is then a
/// dead path, so make sure the drive exists before writing it.
fn ensure_s(prefix: &Path, root: Option<&Path>) -> bool {
    let link = prefix.join("dosdevices/s:");
    if fs::symlink_metadata(&link).is_ok() {
        return true;
    }
    let Some(root) = root else {
        return false;
    };
    let target = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let _ = fs::create_dir_all(prefix.join("dosdevices"));
    let _ = std::os::unix::fs::symlink(target, &link);
    link.exists()
}

// -- launcher readiness / closing ----------------------------------------

fn find_log(prefix: &Path) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for dir in appdata_dirs(prefix) {
        let logs = dir.join("TruckersMP/logs");
        let Ok(entries) = fs::read_dir(&logs) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("log"))
            {
                continue;
            }
            let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
                continue;
            };
            if best.as_ref().is_none_or(|(best, _)| modified > *best) {
                best = Some((modified, path));
            }
        }
    }
    best.map(|(_, path)| path)
}

fn log_stamp(line: &str) -> Option<i64> {
    let re = Regex::new(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z)").ok()?;
    let raw = re.captures(line)?.get(1)?.as_str();
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|stamp| stamp.timestamp())
}

fn launcher_ready(prefix: &Path, since: i64) -> bool {
    let Some(log) = find_log(prefix) else {
        return false;
    };
    let Ok(text) = fs::read_to_string(&log) else {
        return false;
    };
    let cutoff = since - 5;
    text.lines()
        .rev()
        .filter(|line| READY_MARKERS.iter().any(|marker| line.contains(marker)))
        .find_map(log_stamp)
        .is_some_and(|stamp| stamp >= cutoff)
}

fn boot_time() -> f64 {
    fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("btime "))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(0.0)
}

fn ticks_per_second() -> f64 {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks > 0 {
        ticks as f64
    } else {
        100.0
    }
}

fn process_start(pid: i32) -> Option<f64> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = text.rsplit_once(')')?.1;
    let field: f64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    Some(boot_time() + field / ticks_per_second())
}

fn process_cmdline(pid: i32) -> Option<String> {
    let raw = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(String::from_utf8_lossy(&raw).replace('\0', " "))
}

fn alive(pid: i32) -> bool {
    let Ok(text) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(rest) = text.rsplit_once(')').map(|(_, rest)| rest.trim_start()) else {
        return false;
    };
    !rest.starts_with('Z')
}

fn own_pids() -> [i32; 2] {
    [std::process::id() as i32, unsafe { libc::getppid() }]
}

fn wrappers() -> [&'static str; 6] {
    [
        "proton",
        "unshare",
        "steam.exe",
        "waitforexitandrun",
        "python3",
        "steam-mp-launch",
    ]
}

/// Wine processes carry the prefix in `WINEPREFIX`/`STEAM_COMPAT_DATA_PATH`;
/// their command lines are the same `C:\users\steamuser\…` in every prefix, so
/// the environment is the only thing that tells two installs apart (measured:
/// an idle prefix reported `running:true` only because the user's launcher in
/// the real 227300 prefix was open).
fn owned_by_prefix(pid: i32, prefix: &Path) -> bool {
    fs::read(format!("/proc/{pid}/environ"))
        .is_ok_and(|env| String::from_utf8_lossy(&env).contains(prefix.to_string_lossy().as_ref()))
}

fn launcher_pids(prefix: &Path, since: Option<i64>) -> Vec<i32> {
    let own = own_pids();
    let cutoff = since.map(|since| since as f64 - 5.0).unwrap_or(0.0);
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut pids = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse().ok()) else {
            continue;
        };
        if own.contains(&pid) {
            continue;
        }
        let Some(cmd) = process_cmdline(pid) else {
            continue;
        };
        let low = cmd.to_ascii_lowercase();
        if !low.contains("truckersmp-launcher.exe") {
            continue;
        }
        if wrappers().iter().any(|word| low.contains(word)) {
            continue;
        }
        // The Electron process uses a C:\ path, never the unix prefix.
        if !prefix.as_os_str().is_empty() && !owned_by_prefix(pid, prefix) {
            continue;
        }
        if since.is_some() && process_start(pid).is_none_or(|start| start < cutoff) {
            continue;
        }
        pids.push(pid);
    }
    pids.sort_unstable();
    pids
}

fn terminate(pids: &[i32], force: bool) -> usize {
    if pids.is_empty() {
        return 0;
    }
    if !force {
        for pid in pids {
            unsafe {
                libc::kill(*pid, libc::SIGTERM);
            }
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline && pids.iter().any(|pid| alive(*pid)) {
            thread::sleep(Duration::from_millis(50));
        }
    }
    for pid in pids {
        if force || alive(*pid) {
            unsafe {
                libc::kill(*pid, libc::SIGKILL);
            }
        }
    }
    pids.len()
}

// -- list helpers ---------------------------------------------------------

fn unique_label(existing: &[lists::Entry], wanted: &str) -> String {
    if !existing.iter().any(|entry| entry.label == wanted) {
        return wanted.to_owned();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{wanted} {index}");
        if !existing.iter().any(|entry| entry.label == candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn game_path(game_exe: &str, root: Option<&Path>, appid: &str) -> PathBuf {
    if !game_exe.is_empty() {
        return PathBuf::from(game_exe);
    }
    root.map(|root| PathBuf::from(games::find_exe(root, appid)))
        .unwrap_or_default()
}

fn prefix_for(compatdata: &str, root: Option<&Path>, appid: &str) -> (String, PathBuf) {
    let compatdata = if !compatdata.is_empty() {
        compatdata.to_owned()
    } else {
        root.map(|root| {
            root.join("steamapps/compatdata")
                .join(appid)
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_default()
    };
    let prefix = if compatdata.is_empty() {
        PathBuf::new()
    } else {
        protons::prefix_of(Path::new(&compatdata))
    };
    (compatdata, prefix)
}

fn has_s(prefix: &Path) -> bool {
    prefix.join("dosdevices/s:").exists()
}

/// Python `_game_path`: fall back to the setup's default C: path when the
/// game executable is unknown.  Without a real `S:` drive the path has to go
/// through `Z:`, otherwise the JSON points at nothing.
fn game_windows_path(kind: &str, game: &str, root: Option<&Path>, s_mapped: bool) -> String {
    let win = windows_game_path(Path::new(game), if s_mapped { root } else { None });
    if win.is_empty() && kind != "trucksbook" {
        return TRUCKERSMP_GAME_EXE.to_owned();
    }
    win
}

fn trucksbook_game(game_exe: &str, root: Option<&Path>, appid: &str) -> PathBuf {
    if !game_exe.is_empty() {
        return PathBuf::from(game_exe);
    }
    PathBuf::from(find_trucksbook_game_exe(root, appid))
}

// -- commands -------------------------------------------------------------

/// Everything the dialogs need on open: detected download, existing install,
/// game executable and its Windows spelling.
pub fn detect(
    root: Option<&Path>,
    appid: &str,
    kind: &str,
    compatdata: &str,
    game_exe: &str,
) -> Value {
    let (_compatdata, prefix) = prefix_for(compatdata, root, appid);
    let downloads = downloads_dir();
    let installer = if kind == "trucksbook" {
        find_trucksbook_installer(Some(&downloads))
    } else {
        find_downloaded_installer(Some(&downloads))
    };
    let s_mapped = has_s(&prefix);
    let game = if kind == "trucksbook" {
        trucksbook_game(game_exe, root, appid)
            .to_string_lossy()
            .into_owned()
    } else {
        game_path(game_exe, root, appid)
            .to_string_lossy()
            .into_owned()
    };
    let game_win = game_windows_path(kind, &game, root, s_mapped);
    let launcher = find_truckersmp_launcher(&prefix);
    let client = find_trucksbook_client(&prefix);
    // The stub `TruckersMP-Launcher.exe` appears ~17 s before Squirrel is done
    // (see `install_state`), so "the exe exists" is not "it is installed".
    let (complete, installing, version) = install_state(&prefix);
    let installed = if kind == "trucksbook" {
        !client.is_empty()
    } else {
        complete
    };
    json!({
        "installer": installer,
        "downloads": downloads.to_string_lossy(),
        "installed": installed,
        "installing": if kind == "trucksbook" { false } else { installing },
        "version": version,
        "launcher": launcher,
        "client": client,
        "game_exe": game,
        "game_exe_win": game_win,
        "has_s": s_mapped,
        "prefix": prefix.to_string_lossy(),
        "compatdata": _compatdata,
        "steam_root": root.map(|root| root.to_string_lossy().into_owned()).unwrap_or_default(),
    })
}

/// Just the newest downloaded setup (the 1 s auto-detect timer).
pub fn find_installer(kind: &str) -> Value {
    let path = if kind == "trucksbook" {
        find_trucksbook_installer(None)
    } else {
        find_downloaded_installer(None)
    };
    json!({ "path": path })
}

pub fn prepare(path: &str) -> Result<Value, String> {
    clean_stale_temp_dirs();
    let (exe, temp) = resolve_installer(path)?;
    Ok(json!({ "exe": exe, "temp_dir": temp }))
}

/// Is the fresh launcher running and done verifying files?
pub fn probe(
    root: Option<&Path>,
    appid: &str,
    kind: &str,
    game_exe: &str,
    compatdata: &str,
    since: i64,
) -> Value {
    let (_compatdata, prefix) = prefix_for(compatdata, root, appid);
    let game = if kind == "trucksbook" {
        PathBuf::from(find_trucksbook_game_exe(root, appid))
    } else {
        game_path(game_exe, root, appid)
    };
    let launcher = find_truckersmp_launcher(&prefix);
    let client = find_trucksbook_client(&prefix);
    let (complete, installing, version) = install_state(&prefix);
    let running = !launcher_pids(&prefix, None).is_empty();
    // Keep the launcher log as the only "ready" source: after `--squirrel-install`
    // Squirrel kills the launcher at +16 s, and a `--silent` setup never starts
    // it at all, so "no launcher running" must not be mistaken for success.
    let ready = !launcher.is_empty() && launcher_ready(&prefix, since) && !installing;
    json!({
        "found": !launcher.is_empty() || !client.is_empty(),
        "path": if kind == "trucksbook" { client } else { launcher },
        "running": running,
        "ready": ready,
        "installing": if kind == "trucksbook" { false } else { installing },
        "complete": if kind == "trucksbook" { true } else { complete },
        "version": version,
        "prefix": prefix.to_string_lossy(),
        "game_path": game_windows_path(kind, &game.to_string_lossy(), root, has_s(&prefix)),
    })
}

/// Python `close_truckersmp_launcher`: `since` limits it to this install run.
pub fn close(compatdata: &str, root: Option<&Path>, appid: &str, since: i64, force: bool) -> Value {
    let (_compatdata, prefix) = prefix_for(compatdata, root, appid);
    let pids = launcher_pids(&prefix, (since > 0).then_some(since));
    let closed = terminate(&pids, force);
    json!({ "closed": closed, "pids": pids })
}

/// Set the game path and add the launcher/sidecar (Python `_add_path`).
#[allow(clippy::too_many_arguments)]
pub fn finish(
    paths: &Paths,
    root: Option<&Path>,
    appid: &str,
    kind: &str,
    game_exe: &str,
    compatdata: &str,
    manual: &str,
) -> Result<Value, String> {
    if appid.is_empty() {
        return Err("no game appid".to_owned());
    }
    let (_compatdata, prefix) = prefix_for(compatdata, root, appid);
    if prefix.as_os_str().is_empty() {
        return Err("no Proton prefix for this game".to_owned());
    }
    let game = if kind == "trucksbook" {
        trucksbook_game(game_exe, root, appid)
    } else {
        game_path(game_exe, root, appid)
    };
    if kind == "truckersmp" {
        ensure_s(&prefix, root);
    }
    // Only trust `S:\` when the drive really exists now (ensure_s may have
    // failed, e.g. a read-only prefix).
    let s_mapped = has_s(&prefix);
    let game_path_win = game_windows_path(kind, &game.to_string_lossy(), root, s_mapped);

    // Python sets the path before looking for the launcher.
    let (path_set, config) = if kind == "trucksbook" {
        (true, String::new())
    } else {
        set_truckersmp_game_path(&prefix, &game_path_win)
    };

    let (list_file, label) = if kind == "trucksbook" {
        (paths.extras_file(appid), TRUCKSBOOK_LABEL)
    } else {
        (paths.launchers_file(appid), TRUCKERSMP_LABEL)
    };
    let found_path = if !manual.is_empty() {
        manual.to_owned()
    } else if kind == "trucksbook" {
        find_trucksbook_client(&prefix)
    } else {
        find_truckersmp_launcher(&prefix)
    };
    if found_path.is_empty() || !Path::new(&found_path).is_file() {
        return Ok(json!({
            "found": false,
            "game_path": game_path_win,
            "game_path_set": path_set,
            "config": config,
            "list_file": list_file.to_string_lossy(),
        }));
    }

    // `add_launcher`: exact path dedupe, otherwise a unique label.
    let entries = lists::read_entries(&list_file);
    let exists = entries.iter().any(|entry| entry.path == found_path);
    let added_label = if exists {
        entries
            .iter()
            .find(|entry| entry.path == found_path)
            .map(|entry| entry.label.clone())
            .unwrap_or_else(|| label.to_owned())
    } else {
        let unique = unique_label(&entries, label);
        lists::add(&list_file, &unique, &found_path)?;
        unique
    };

    Ok(json!({
        "found": true,
        "path": found_path,
        "label": added_label,
        "game_path": game_path_win,
        "game_path_set": path_set,
        "config": config,
        "added": !exists,
        "list_file": list_file.to_string_lossy(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("mp-install-{name}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn prefix_with_log(dir: &Path, line: &str) -> PathBuf {
        let prefix = dir.join("pfx");
        let logs = prefix.join("drive_c/users/u/AppData/Roaming/TruckersMP/logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("today.log"), format!("{line}\n")).unwrap();
        prefix
    }

    #[test]
    fn readiness_uses_the_python_marker_only() {
        let dir = scratch("ready");
        let stamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
        let since = chrono::Utc::now().timestamp();
        let prefix = prefix_with_log(
            &dir,
            &format!("{stamp} - renderer - [DEBUG] Verify files operation finished"),
        );
        assert!(launcher_ready(&prefix, since - 60));
        assert!(!launcher_ready(&prefix, since + 60), "old marker");

        // 1.3.x writes this while verifying; Python does not treat it as
        // readiness and neither do we (the launcher must stay open).
        let other = dir.join("other");
        let prefix = prefix_with_log(
            &other,
            &format!("{stamp} - renderer - [DEBUG] useUpdateFiles unmounted"),
        );
        assert!(!launcher_ready(&prefix, since - 60));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn game_path_uses_s_drive_inside_steam_root() {
        let dir = scratch("paths");
        let root = dir.join("Steam");
        let exe = root.join("steamapps/common/ETS2/bin/win_x64/eurotrucks2.exe");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"x").unwrap();
        assert_eq!(
            windows_game_path(&exe, Some(&root)),
            r"S:\steamapps\common\ETS2\bin\win_x64\eurotrucks2.exe"
        );
        assert!(windows_game_path(&exe, Some(&dir.join("elsewhere"))).starts_with("Z:\\"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn truckersmp_options_keep_other_keys() {
        let dir = scratch("options");
        let prefix = dir.join("pfx");
        let config =
            prefix.join("drive_c/users/u/AppData/Roaming/TruckersMP/launcher-options.json");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::create_dir_all(prefix.join("dosdevices/s:")).unwrap();
        fs::write(
            &config,
            r#"{"theme":"dark","games":{"ets2":{"game":"ets2","path":"C:\\old.exe","consoleOpts":[1]}}}"#,
        )
        .unwrap();
        let (ok, path) = set_truckersmp_game_path(&prefix, r"S:\steamapps\ets2.exe");
        assert!(ok && path == config.to_string_lossy());
        let data: Value = serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(data["games"]["ets2"]["path"], r"S:\steamapps\ets2.exe");
        assert_eq!(data["games"]["ets2"]["consoleOpts"][0], 1);
        assert_eq!(data["theme"], "dark");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn zip_names_reject_escapes() {
        assert!(safe_name("setup/TruckersMP-Setup.exe").is_ok());
        assert!(safe_name("a/../setup.exe").is_ok());
        assert!(safe_name("../setup.exe").is_err());
        assert!(safe_name("/etc/passwd").is_err());
        assert!(safe_name(r"C:\setup.exe").is_err());
    }

    #[test]
    fn installer_scoring_prefers_setup() {
        let dir = scratch("scoring");
        let nested = dir.join("sub");
        fs::create_dir_all(&nested).unwrap();
        fs::write(dir.join("TruckersMP-Launcher.exe"), b"x").unwrap();
        fs::write(nested.join("TruckersMP-Setup.exe"), b"xx").unwrap();
        assert_eq!(
            Path::new(&installer_exe(&dir)).file_name().unwrap(),
            "TruckersMP-Setup.exe"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn downloads_detection_uses_the_newest_match() {
        let dir = scratch("downloads");
        fs::write(dir.join("TruckersMP-Setup.exe"), b"a").unwrap();
        fs::write(dir.join("TruckersMP-old.zip"), b"b").unwrap();
        fs::write(dir.join("other.exe"), b"c").unwrap();
        let newest = find_downloaded_installer(Some(&dir));
        assert!(newest.ends_with("TruckersMP-old.zip") || newest.ends_with("TruckersMP-Setup.exe"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn game_path_uses_z_drive_when_s_is_missing() {
        let dir = scratch("paths-z");
        let root = dir.join("Steam");
        let exe = root.join("steamapps/common/ETS2/bin/win_x64/eurotrucks2.exe");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"x").unwrap();
        let game = exe.to_string_lossy().into_owned();
        assert_eq!(
            game_windows_path("truckersmp", &game, Some(&root), true),
            r"S:\steamapps\common\ETS2\bin\win_x64\eurotrucks2.exe"
        );
        // No S: in the prefix -> a Z: path that always resolves, never a dead S:.
        let fallback = game_windows_path("truckersmp", &game, Some(&root), false);
        assert!(fallback.starts_with("Z:\\"), "{fallback}");
        assert!(fallback.ends_with(r"eurotrucks2.exe"));
        // An unknown game keeps the setup's default C: path.
        assert_eq!(
            game_windows_path("truckersmp", "", Some(&root), false),
            TRUCKERSMP_GAME_EXE
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn launcher_lookup_prefers_the_exact_exe() {
        let dir = scratch("launcher-rank");
        let prefix = dir.join("pfx");
        let local = prefix.join("drive_c/users/u/AppData/Local/TruckersMP");
        let roaming = prefix.join("drive_c/users/u/AppData/Roaming/TruckersMP/old");
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(&roaming).unwrap();
        fs::write(local.join("TruckersMP-Launcher.exe"), b"exact").unwrap();
        fs::write(roaming.join("TruckersMP-Launcher-2.exe"), b"old").unwrap();
        let found = find_truckersmp_launcher(&prefix);
        assert_eq!(
            Path::new(&found).file_name().unwrap(),
            "TruckersMP-Launcher.exe"
        );
        // Without the exact name the remaining match is still found.
        fs::remove_file(local.join("TruckersMP-Launcher.exe")).unwrap();
        let found = find_truckersmp_launcher(&prefix);
        assert!(
            found.ends_with("TruckersMP-Launcher-2.exe"),
            "unexpected: {found}"
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn options_are_looked_up_in_roaming_first() {
        let dir = scratch("options-order");
        let prefix = dir.join("pfx");
        let roaming = prefix.join("drive_c/users/u/AppData/Roaming/TruckersMP");
        let local = prefix.join("drive_c/users/u/AppData/Local/TruckersMP");
        fs::create_dir_all(&roaming).unwrap();
        fs::create_dir_all(&local).unwrap();
        fs::write(roaming.join("launcher-options.json"), b"{}").unwrap();
        fs::write(local.join("launcher-options.json"), b"{}").unwrap();
        let found = find_options(&prefix).unwrap();
        assert!(
            found.starts_with(&roaming),
            "unexpected: {}",
            found.display()
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn only_old_installer_temp_dirs_are_stale() {
        assert!(is_stale_temp(
            "mp-truckersmp-123-1",
            Duration::from_secs(7 * 3600)
        ));
        assert!(!is_stale_temp(
            "mp-truckersmp-123-1",
            Duration::from_secs(60)
        ));
        assert!(!is_stale_temp(
            "truckersmp-old",
            Duration::from_secs(7 * 3600)
        ));
    }
}
