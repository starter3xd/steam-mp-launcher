//! Filesystem layout and per-game file names.

use std::env;
use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn nonempty(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

pub fn valid_appid(appid: &str) -> bool {
    !appid.is_empty()
        && appid.bytes().all(|b| b.is_ascii_digit())
        && appid.parse::<u64>().map(|n| n > 0).unwrap_or(false)
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub conf_dir: PathBuf,
    pub conf: PathBuf,
    pub launchers: PathBuf,
    pub extras: PathBuf,
    pub saved: PathBuf,
    pub logs: PathBuf,
}

impl Paths {
    pub fn detect() -> Self {
        let home = home();
        let config_base = nonempty("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let state_base = nonempty("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/state"));
        let conf_dir = config_base.join("steam-mp-launcher");

        let pick = |modern: &str, legacy: &str, default: PathBuf| {
            nonempty(modern)
                .or_else(|| nonempty(legacy))
                .map(PathBuf::from)
                .unwrap_or(default)
        };

        let conf = pick(
            "MP_CONF",
            "TRUCKERSMP_CONF",
            conf_dir.join("steam-mp-launcher.conf"),
        );
        let launchers = pick(
            "MP_LIST",
            "TRUCKERSMP_LIST",
            conf_dir.join("launchers.list"),
        );
        let extras = pick(
            "MP_EXTRAS",
            "TRUCKERSMP_EXTRAS",
            conf_dir.join("extras.list"),
        );
        let saved = pick("MP_SAVED", "TRUCKERSMP_SAVED", conf_dir.join("launcher"));

        Self {
            conf_dir,
            conf,
            launchers,
            extras,
            saved,
            logs: state_base.join("steam-mp-launcher/logs"),
        }
    }

    pub fn shared(&self, which: &str) -> &Path {
        if which == "extras" {
            &self.extras
        } else {
            &self.launchers
        }
    }

    /// `<dir>/launcher-<appid>`, `launchers-<appid>.list`, `extras-<appid>.list`.
    pub fn game_file_named(shared: &Path, which: &str, appid: &str) -> Option<PathBuf> {
        if !valid_appid(appid) {
            return None;
        }
        let name = match which {
            "launcher" => format!("launcher-{appid}"),
            "launchers" => format!("launchers-{appid}.list"),
            "extras" => format!("extras-{appid}.list"),
            _ => return None,
        };
        Some(shared.parent()?.join(name))
    }

    pub fn effective_named(shared: &Path, which: &str, appid: &str) -> PathBuf {
        Self::game_file_named(shared, which, appid)
            .filter(|path| path.exists())
            .unwrap_or_else(|| shared.to_path_buf())
    }

    pub fn launchers_file(&self, appid: &str) -> PathBuf {
        Self::effective_named(&self.launchers, "launchers", appid)
    }

    pub fn extras_file(&self, appid: &str) -> PathBuf {
        Self::effective_named(&self.extras, "extras", appid)
    }

    pub fn game_saved(&self, appid: &str) -> Option<PathBuf> {
        Self::game_file_named(&self.saved, "launcher", appid)
    }
}

pub fn atomic_write(path: &Path, text: &str) -> std::io::Result<()> {
    atomic_write_bytes(path, text.as_bytes())
}

pub fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mode = std::fs::metadata(path).ok().map(|m| {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode()
    });
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id(),
        stamp
    ));
    std::fs::write(&tmp, bytes)?;
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    }
    if let Err(err) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}
