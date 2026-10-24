//! Steam compatibility-tool entry point.
//!
//! Steam calls this with `run <game.exe>`, `waitforexitandrun <game.exe>`,
//! `runinprefix <exe>`, `getcompatpath <path>` or `getnativepath <path>`.
//! The picker is the C++ UI in `--pick` mode; launching and the session are
//! handled by the Rust backend.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mp_backend::{clean_proton_env, handle, steam_root, Ctx};
use serde_json::{json, Value};

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn call(ctx: &mut Ctx, cmd: &str, extra: Value) -> Value {
    let mut request = extra;
    request["cmd"] = json!(cmd);
    handle(ctx, &request)
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn appid() -> String {
    for key in ["SteamAppId", "SteamGameId"] {
        let value = env(key);
        if !value.is_empty() {
            return value;
        }
    }
    String::new()
}

fn is_helper(appid: &str, target: &str) -> bool {
    looks_like_helper(appid, target, steam_root().as_deref())
}

/// Resolve `.`/`..` without touching the filesystem (Steam hands over paths
/// like `<root>/ubuntu12_32/../bin/x` where the middle directory may be gone).
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn looks_like_helper(appid: &str, target: &str, root: Option<&Path>) -> bool {
    if appid == "0" {
        return true;
    }
    if target.is_empty() {
        return false;
    }
    let base = Path::new(target)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        base.as_str(),
        "d3ddriverquery32.exe" | "d3ddriverquery64.exe" | "iscriptevaluator.exe"
    ) {
        return true;
    }
    if target.to_ascii_lowercase().contains("legacycompat") {
        return true;
    }
    // Steam also ships helpers in <root>/bin and hands them over through odd
    // `ubuntu12_32/../bin/x` paths; they are never a game.
    let real = std::fs::canonicalize(target).unwrap_or_else(|_| lexical(Path::new(target)));
    root.is_some_and(|root| real.starts_with(root.join("bin")))
}

fn ui_path() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("MP_UI") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join("steam-mp-launcher"),
        dir.join("ui/steam-mp-launcher"),
        dir.join("../../../ui/steam-mp-launcher"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

fn picker_state(ctx: &mut Ctx, appid: &str, game_exe: &str) -> Value {
    let mut args = json!({});
    if !appid.is_empty() {
        args["appid"] = json!(appid);
    }
    if !game_exe.is_empty() {
        args["game_exe"] = json!(game_exe);
    }
    call(ctx, "picker", args)
}

/// The picker helper prints the chosen program on the first line and
/// `--no-sidecars` when its sidecar switch is off.
fn parse_pick(stdout: &[u8]) -> (String, bool) {
    let text = String::from_utf8_lossy(stdout).into_owned();
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let value = lines.next().unwrap_or("").to_owned();
    let sidecars = !lines.any(|line| line == "--no-sidecars");
    (value, sidecars)
}

#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Chosen,
    Cancelled,
    Failed,
}

fn run_picker(ui: &Path, appid: &str) -> (Pick, String, bool) {
    let mut command = Command::new(ui);
    command.arg("--pick");
    if !appid.is_empty() {
        command.arg("--appid").arg(appid);
    }
    // Steam's LD_PRELOAD/LD_LIBRARY_PATH point at the runtime and the overlay;
    // they make a Qt process load the wrong libraries.  Keep them for Proton
    // (this runner launches the game), but not for the picker.
    clean_qt_env(ui, &mut command);
    command.stdin(Stdio::null());
    match command.output() {
        Ok(output) => {
            let (chosen, sidecars) = parse_pick(&output.stdout);
            if output.status.success() && !chosen.is_empty() {
                (Pick::Chosen, chosen, sidecars)
            } else if output.status.code() == Some(1) {
                (Pick::Cancelled, String::new(), true)
            } else {
                eprintln!(
                    "mp-run: picker failed ({}): {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                (Pick::Failed, String::new(), true)
            }
        }
        Err(err) => {
            eprintln!("mp-run: could not start the C++ picker: {err}");
            (Pick::Failed, String::new(), true)
        }
    }
}

/// The bundle keeps the UI in `usr/bin` and the Qt libraries in
/// `usr/lib`; `mp-run` strips Steam's loader env for the UI, so the bundle's
/// own paths have to be put back (a source install has no `usr/bin`).
fn clean_qt_env(ui: &Path, command: &mut Command) {
    command.env_remove("LD_PRELOAD");
    command.env_remove("LD_LIBRARY_PATH");
    command.env_remove("LC_ALL");
    let Some(bin) = ui.parent() else { return };
    if bin.file_name() != Some(std::ffi::OsStr::new("bin")) {
        return;
    }
    let Some(root) = bin.parent() else { return };
    let lib = root.join("lib");
    if !lib.is_dir() {
        return;
    }
    command.env("LD_LIBRARY_PATH", &lib);
    let plugins = root.join("plugins");
    if plugins.is_dir() {
        command.env("QT_PLUGIN_PATH", &plugins);
        command.env("QT_QPA_PLATFORM_PLUGIN_PATH", plugins.join("platforms"));
    }
}

fn run_confirm(ui: &Path, label: &str, game: &str, seconds: i64) -> i32 {
    let mut command = Command::new(ui);
    command.args([
        "--confirm",
        "--label",
        label,
        "--game-name",
        game,
        "--seconds",
        &seconds.to_string(),
    ]);
    clean_qt_env(ui, &mut command);
    match command.status() {
        Ok(status) => status.code().unwrap_or(0),
        Err(err) => {
            eprintln!("mp-run: confirm card failed: {err}");
            0 // never block a launch because the optional card failed
        }
    }
}

#[derive(Clone)]
struct Choice {
    exe: String,
    sidecars: bool,
}

fn choice(exe: String) -> Choice {
    Choice {
        exe,
        sidecars: true,
    }
}

/// `GUI_CANCEL=game` means "closing the picker runs the plain game"; anything
/// else stops the launch.
fn cancelled(state: &Value) -> Result<Option<Choice>, String> {
    let game = state["game"]["exe"].as_str().unwrap_or("");
    if state["cancel_mode"].as_str().unwrap_or("abort") == "game" && !game.is_empty() {
        Ok(Some(choice(game.to_owned())))
    } else {
        Ok(None)
    }
}

fn choose(ctx: &mut Ctx, appid: &str, game_exe: &str) -> Result<Option<Choice>, String> {
    if STOP.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let state = picker_state(ctx, appid, game_exe);
    if state["ok"].as_bool() != Some(true) {
        return Err(state["error"]
            .as_str()
            .unwrap_or("picker state failed")
            .to_owned());
    }
    let saved = state["saved"].as_str().unwrap_or("").to_owned();
    let launcher_gui = state["launcher_gui"].as_str().unwrap_or("ask");
    if launcher_gui == "saved" && !saved.is_empty() && Path::new(&saved).is_file() {
        let delay = state["confirm_delay"].as_i64().unwrap_or(0);
        if delay > 0 {
            if let Some(ui) = ui_path() {
                let label = Path::new(&saved)
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Launcher");
                let game = state["game"]["name"].as_str().unwrap_or("Game");
                match run_confirm(&ui, label, game, delay) {
                    1 => return cancelled(&state),
                    2 => {} // fall through to the full picker
                    _ => return Ok(Some(choice(saved))),
                }
            } else {
                return Ok(Some(choice(saved)));
            }
        } else {
            return Ok(Some(choice(saved)));
        }
    }

    if let Some(ui) = ui_path() {
        match run_picker(&ui, appid) {
            (Pick::Chosen, chosen, sidecars) => {
                let exe = if chosen == "__GAME__" || chosen.is_empty() {
                    state["game"]["exe"].as_str().unwrap_or("").to_owned()
                } else {
                    chosen
                };
                return Ok(Some(Choice { exe, sidecars }));
            }
            (Pick::Cancelled, _, _) => return cancelled(&state),
            (Pick::Failed, _, _) => {}
        }
    } else {
        eprintln!("mp-run: C++ picker not found (set MP_UI)");
    }
    if STOP.load(Ordering::SeqCst) {
        return Ok(None);
    }

    // Non-interactive fallback: a valid saved choice, then the plain game.
    if !saved.is_empty() && Path::new(&saved).is_file() {
        return Ok(Some(choice(saved)));
    }
    let fallback = state["fallback_to_game"].as_str().unwrap_or("1") != "0";
    let game = state["game"]["exe"].as_str().unwrap_or("").to_owned();
    if fallback && !game.is_empty() {
        Ok(Some(choice(game)))
    } else {
        Ok(None)
    }
}

fn proton(ctx: &mut Ctx) -> Result<String, String> {
    let reply = call(ctx, "protons", json!({}));
    let current = reply["current"].as_str().unwrap_or("");
    if current.is_empty() {
        Err("no Proton installation found".to_owned())
    } else {
        Ok(current.to_owned())
    }
}

fn compatdata(ctx: &mut Ctx, appid: &str) -> String {
    let from_env = env("STEAM_COMPAT_DATA_PATH");
    if !from_env.is_empty() {
        return from_env;
    }
    let state = picker_state(ctx, appid, "");
    state["game"]["compatdata"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn proton_command(proton: &str, data: &str, appid: &str, mode: &str, target: &str) -> Command {
    let mut command = Command::new(proton);
    command.arg(mode).arg(target);
    clean_proton_env(&mut command);
    if !data.is_empty() {
        command.env("STEAM_COMPAT_DATA_PATH", data);
    }
    let root = env("STEAM_COMPAT_CLIENT_INSTALL_PATH");
    if !root.is_empty() {
        command.env("STEAM_COMPAT_CLIENT_INSTALL_PATH", root);
    }
    if !appid.is_empty() {
        command.env("SteamAppId", appid);
        command.env("SteamGameId", appid);
    }
    command
}

fn direct(ctx: &mut Ctx, mode: &str, target: &str) -> Result<ExitStatus, String> {
    let proton = proton(ctx)?;
    let appid = appid();
    let data = compatdata(ctx, &appid);
    proton_command(&proton, &data, &appid, mode, target)
        .status()
        .map_err(|e| e.to_string())
}

fn spawn_tray(ui: &Path, appid: &str, state: &Value, chosen: &str) -> bool {
    let log = state["log"].as_str().unwrap_or("");
    if log.is_empty() {
        return false;
    }
    let game = state["game"].as_str().unwrap_or("Game");
    let label = Path::new(chosen)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Launcher");
    let mut command = Command::new(ui);
    command.args([
        "--tray",
        "--appid",
        appid,
        "--log",
        log,
        "--game-name",
        game,
        "--label",
        label,
        "--parent-pid",
        &std::process::id().to_string(),
    ]);
    clean_qt_env(ui, &mut command);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    command.spawn().is_ok()
}

fn wait_session(ctx: &mut Ctx, id: u64) -> i32 {
    let mut stopped = false;
    loop {
        if STOP.load(Ordering::SeqCst) && !stopped {
            stopped = true;
            let _ = call(ctx, "stop_session", json!({ "id": id }));
        }
        let state = call(ctx, "session", json!({ "id": id }));
        if state["done"].as_bool() == Some(true) {
            let error = state["error"].as_str().unwrap_or("");
            if !error.is_empty() {
                eprintln!("mp-run: {error}");
                return 1;
            }
            if stopped {
                return 143;
            }
            return state["rc"].as_i64().unwrap_or(1) as i32;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn main() {
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let verb = args
        .first()
        .map(String::as_str)
        .unwrap_or("waitforexitandrun");
    let target = args.get(1).cloned().unwrap_or_default();
    let mut ctx = Ctx::new();
    let appid = appid();

    let rc = match verb {
        "getcompatpath" | "getnativepath" => {
            if target.is_empty() {
                eprintln!("mp-run: {verb} requires a path");
                1
            } else {
                match direct(&mut ctx, verb, &target) {
                    Ok(status) => status.code().unwrap_or(1),
                    Err(err) => {
                        eprintln!("mp-run: {err}");
                        1
                    }
                }
            }
        }
        "runinprefix" => match direct(&mut ctx, "runinprefix", &target) {
            Ok(status) => status.code().unwrap_or(1),
            Err(err) => {
                eprintln!("mp-run: {err}");
                1
            }
        },
        "run" | "waitforexitandrun" => {
            if is_helper(&appid, &target) {
                match direct(&mut ctx, "run", &target) {
                    Ok(status) => status.code().unwrap_or(1),
                    Err(err) => {
                        eprintln!("mp-run: {err}");
                        1
                    }
                }
            } else {
                match choose(&mut ctx, &appid, &target) {
                    Ok(None) => 0, // cancelled: Steam sees a clean no-op
                    Ok(Some(chosen)) if !STOP.load(Ordering::SeqCst) => {
                        let reply = call(
                            &mut ctx,
                            "launch",
                            json!({
                                "appid": appid,
                                "exe": chosen.exe,
                                "game_exe": target,
                                "sidecars": chosen.sidecars,
                                "remember": false
                            }),
                        );
                        if reply["ok"].as_bool() != Some(true) {
                            eprintln!(
                                "mp-run: {}",
                                reply["error"].as_str().unwrap_or("launch failed")
                            );
                            1
                        } else {
                            if let Some(ui) = ui_path() {
                                spawn_tray(&ui, &appid, &reply["session"], &chosen.exe);
                            }
                            wait_session(&mut ctx, reply["id"].as_u64().unwrap_or(0))
                        }
                    }
                    Ok(Some(_)) => 143,
                    Err(err) => {
                        eprintln!("mp-run: {err}");
                        1
                    }
                }
            }
        }
        other => {
            eprintln!("mp-run: unknown verb {other}");
            1
        }
    };
    std::process::exit(rc);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mp-run-{name}-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn steam_bin_helpers_are_not_games() {
        let dir = scratch("helper");
        let root = dir.join("Steam");
        let helper = root.join("bin/steamerrorreporter.exe");
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(&helper, b"MZ").unwrap();
        assert!(looks_like_helper("227300", &helper.to_string_lossy(), Some(&root)));
        // Odd path Steam likes to hand over: resolved before the check.
        let odd = root.join("ubuntu12_32/../bin/steamerrorreporter.exe");
        assert!(looks_like_helper("227300", &odd.to_string_lossy(), Some(&root)));
        // A real game next to it is not a helper.
        let game = root.join("steamapps/common/ETS2/bin/win_x64/eurotrucks2.exe");
        std::fs::create_dir_all(game.parent().unwrap()).unwrap();
        std::fs::write(&game, b"MZ").unwrap();
        assert!(!looks_like_helper("227300", &game.to_string_lossy(), Some(&root)));
        let other = dir.join("elsewhere/tool.exe");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, b"MZ").unwrap();
        assert!(!looks_like_helper("227300", &other.to_string_lossy(), Some(&root)));
        assert!(looks_like_helper("0", "/games/eurotrucks2.exe", Some(&root)));
        assert!(looks_like_helper("227300", "/steam/legacycompat/run.exe", None));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_picker_output_carries_the_sidecar_switch() {
        assert_eq!(
            parse_pick(b"/games/launcher.exe\n").0,
            "/games/launcher.exe"
        );
        assert!(parse_pick(b"/games/launcher.exe\n").1);
        assert!(!parse_pick(b"/games/launcher.exe\n--no-sidecars\n").1);
        assert_eq!(
            parse_pick(b"  /games/launcher.exe  \n").0,
            "/games/launcher.exe"
        );
    }

    #[test]
    fn cancel_mode_game_returns_the_plain_game() {
        let state = json!({
            "game": {"exe": "/games/eurotrucks2.exe"},
            "cancel_mode": "game",
        });
        let chosen = cancelled(&state).unwrap().map(|choice| choice.exe);
        assert_eq!(chosen.as_deref(), Some("/games/eurotrucks2.exe"));
        let abort = json!({
            "game": {"exe": "/games/eurotrucks2.exe"},
            "cancel_mode": "abort",
        });
        assert!(cancelled(&abort).unwrap().is_none());
        // Nothing to run when Steam did not hand over a game path.
        let empty = json!({"game": {"exe": ""}, "cancel_mode": "game"});
        assert!(cancelled(&empty).unwrap().is_none());
    }
}
