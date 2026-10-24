//! `launcher` / `launcher-<appid>` handling.

use std::fs;
use std::io::Write;
use std::path::Path;

pub const GAME_SENTINEL: &str = "__GAME__";

pub fn read_saved(game: Option<&Path>, shared: &Path) -> String {
    let path = game.filter(|p| fs::metadata(p).is_ok()).unwrap_or(shared);
    let text = fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .next()
        .unwrap_or("")
        .trim_end_matches('\r')
        .to_owned()
}

pub fn save_choice(target: &Path, value: &str) -> Result<(), String> {
    if value.is_empty() || value == GAME_SENTINEL {
        return Err("nothing to remember".to_owned());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut file = fs::File::create(target).map_err(|e| format!("{}: {e}", target.display()))?;
    writeln!(file, "{value}").map_err(|e| e.to_string())
}

pub fn forget(game: Option<&Path>, shared: &Path) -> Result<(), String> {
    if let Some(game) = game {
        if let Some(parent) = game.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(game, "").map_err(|e| e.to_string())?;
        return Ok(());
    }
    match fs::remove_file(shared) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp(name: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mp-{name}-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn per_game_wins_even_when_empty() {
        let dir = temp("saved");
        let shared = dir.join("launcher");
        let game = dir.join("launcher-227300");
        fs::write(&shared, "/games/one.exe\n").unwrap();
        assert_eq!(read_saved(Some(&game), &shared), "/games/one.exe");
        fs::write(&game, "").unwrap();
        assert_eq!(read_saved(Some(&game), &shared), "");
        save_choice(&game, "/games/two.exe").unwrap();
        assert_eq!(read_saved(Some(&game), &shared), "/games/two.exe");
        forget(Some(&game), &shared).unwrap();
        assert_eq!(read_saved(Some(&game), &shared), "");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
