//! Launching Proton + sidecars and exposing a pollable session snapshot.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct SessionState {
    pub id: u64,
    pub appid: String,
    pub game: String,
    pub exe: String,
    pub proton: String,
    pub log: String,
    pub started: bool,
    pub game_up: bool,
    pub extras_started: Vec<String>,
    pub extras_skipped: Vec<String>,
    pub stopping: bool,
    pub done: bool,
    pub rc: i32,
    pub error: String,
}

#[derive(Clone)]
struct Control {
    state: Arc<Mutex<SessionState>>,
    stop: Arc<AtomicBool>,
    pgid: Arc<Mutex<Option<i32>>>,
    children: Arc<Mutex<Vec<i32>>>,
    done_at: Arc<Mutex<Option<SystemTime>>>,
}

#[derive(Default)]
pub struct Sessions {
    next: AtomicU64,
    map: Mutex<HashMap<u64, Control>>,
}

pub struct LaunchSpec {
    pub appid: String,
    pub game: String,
    pub exe: String,
    pub game_exe: String,
    pub proton: PathBuf,
    pub compatdata: PathBuf,
    pub steam_root: Option<PathBuf>,
    pub extras: Vec<String>,
    /// The picker's "start sidecars" switch for this launch.
    pub sidecars: bool,
    /// `EXTRA_TIMEOUT`: how long a program may take to appear before the next
    /// one starts. `0` (default when the user sets it) disables the wait.
    pub extra_timeout: u64,
    /// `WAIT_FOR_GAME_TIMEOUT`. `None` = `WAIT_FOR_GAME=0`, do not hold the
    /// session for the game; `Some(0)` = no limit, `Some(n)` = n seconds.
    pub wait_limit: Option<u64>,
    pub extra_delay: u64,
    pub logs_dir: PathBuf,
}

/// How long a single wait may take. The config uses two different meanings of
/// `0`, so they are kept apart here instead of overloading one integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Limit {
    /// Do not wait at all (`EXTRA_TIMEOUT=0`).
    None,
    /// Wait without a deadline (`WAIT_FOR_GAME_TIMEOUT=0` = no limit).
    Forever,
    Secs(u64),
}

impl Limit {
    fn start(secs: u64) -> Self {
        if secs == 0 {
            Self::None
        } else {
            Self::Secs(secs)
        }
    }

    fn keep(secs: u64) -> Self {
        if secs == 0 {
            Self::Forever
        } else {
            Self::Secs(secs)
        }
    }

    fn none(self) -> bool {
        matches!(self, Self::None)
    }

    fn deadline(self) -> Option<Instant> {
        match self {
            Self::Secs(secs) => Instant::now().checked_add(Duration::from_secs(secs)),
            Self::None | Self::Forever => None,
        }
    }

    fn label(self) -> String {
        match self {
            Self::None => "no wait".to_owned(),
            Self::Forever => "no limit".to_owned(),
            Self::Secs(secs) => format!("{secs}s"),
        }
    }
}

/// Process names that only wrap the real program. Matching them would make a
/// launcher/game look "up" the moment Proton starts, and would keep a session
/// alive for as long as Steam's own runtime wrappers live (they carry the
/// full game path in their argv and are the tool's ancestors).
const WRAPPERS: [&str; 4] = ["reaper", "steam-mp-launch", "unshare", "steam.exe"];

/// A launcher that hands the game over to Steam may exit a couple of seconds
/// before the game process shows up. This is how long the session stays open
/// waiting for it; once the game is seen, `WAIT_FOR_GAME_TIMEOUT` takes over.
const HANDOFF_GRACE: u64 = 10;

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&self, spec: LaunchSpec) -> Result<(u64, SessionState), String> {
        if !Path::new(&spec.exe).is_file() {
            return Err(format!("chosen program does not exist: {}", spec.exe));
        }
        if !spec.proton.is_file() {
            return Err("no Proton installation found".to_owned());
        }
        fs::create_dir_all(&spec.compatdata).map_err(|e| e.to_string())?;
        fs::create_dir_all(&spec.logs_dir).map_err(|e| e.to_string())?;

        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let safe = spec
            .game
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "._-".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let log = spec.logs_dir.join(format!("mp-{safe}-{}.log", spec.appid));
        let state = SessionState {
            id,
            appid: spec.appid.clone(),
            game: spec.game.clone(),
            exe: spec.exe.clone(),
            proton: spec.proton.to_string_lossy().into_owned(),
            log: log.to_string_lossy().into_owned(),
            started: false,
            game_up: false,
            extras_started: Vec::new(),
            extras_skipped: Vec::new(),
            stopping: false,
            done: false,
            rc: 0,
            error: String::new(),
        };
        let control = Control {
            state: Arc::new(Mutex::new(state.clone())),
            stop: Arc::new(AtomicBool::new(false)),
            pgid: Arc::new(Mutex::new(None)),
            children: Arc::new(Mutex::new(Vec::new())),
            done_at: Arc::new(Mutex::new(None)),
        };
        self.map
            .lock()
            .map_err(|_| "session lock poisoned".to_owned())?
            .insert(id, control.clone());
        self.prune();

        let spec = Arc::new(spec);
        let worker = control.clone();
        let spawned = thread::Builder::new()
            .name(format!("mp-launch-{id}"))
            .spawn(move || run(spec, worker));
        if let Err(err) = spawned {
            if let Ok(mut map) = self.map.lock() {
                map.remove(&id);
            }
            return Err(format!("could not start worker: {err}"));
        }
        Ok((id, state))
    }

    /// Finished sessions stay pollable for a while; without this the map grows
    /// for the whole life of the UI process.
    fn prune(&self) {
        let Ok(mut map) = self.map.lock() else {
            return;
        };
        if map.len() <= 8 {
            return;
        }
        let mut finished: Vec<(u64, SystemTime)> = Vec::new();
        for (id, control) in map.iter() {
            if let Ok(stamp) = control.done_at.lock() {
                if let Some(stamp) = *stamp {
                    finished.push((*id, stamp));
                }
            }
        }
        if finished.len() <= 8 {
            return;
        }
        finished.sort_by_key(|(_, stamp)| *stamp);
        let drop = finished.len() - 8;
        for (id, _) in finished.into_iter().take(drop) {
            map.remove(&id);
        }
    }

    pub fn state(&self, id: u64) -> Option<SessionState> {
        let map = self.map.lock().ok()?;
        let control = map.get(&id)?;
        control.state.lock().ok().map(|state| state.clone())
    }

    pub fn stop(&self, id: u64) -> Result<(), String> {
        let map = self.map.lock().map_err(|_| "session lock poisoned")?;
        let control = map.get(&id).ok_or("unknown session")?;
        control.stop.store(true, Ordering::SeqCst);
        if let Ok(pgid) = control.pgid.lock() {
            if let Some(pgid) = *pgid {
                signal(pgid, libc::SIGTERM);
            }
        }
        let sidecars = control
            .children
            .lock()
            .map(|children| children.clone())
            .unwrap_or_default();
        for pid in &sidecars {
            signal(*pid, libc::SIGTERM);
        }
        if let Ok(mut state) = control.state.lock() {
            state.stopping = true;
        }
        Ok(())
    }
}

fn stamp() -> String {
    use chrono::Local;
    Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// One line of the run log, in the format the log viewer expects.
fn log_line(file: &mut fs::File, text: &str) {
    let _ = writeln!(file, "[{}] {text}", stamp());
}

fn log(file: &mut fs::File, key: &str, value: &str) {
    log_line(file, &format!("{key:<17} : {value}"));
}

fn set<F: FnOnce(&mut SessionState)>(control: &Control, f: F) {
    if let Ok(mut state) = control.state.lock() {
        f(&mut state);
    }
}

/// The log is opened with `O_APPEND` and every child gets a `dup` of that very
/// handle. A second, independent handle (with its own offset) would be
/// overwritten by the child's stdout the moment it writes anything.
fn open_log(path: &Path) -> std::io::Result<fs::File> {
    std::fs::File::create(path)?;
    OpenOptions::new().append(true).open(path)
}

fn base_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase()
}

fn pid_alive(pid: i32) -> bool {
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(text) => text
            .rsplit_once(')')
            .map(|(_, rest)| !rest.trim_start().starts_with('Z'))
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Signal the whole process group when `pid` is its leader (the children are
/// started with `setsid`), otherwise just that pid.
fn signal(pid: i32, signal_number: i32) {
    unsafe {
        if libc::getpgid(pid) == pid {
            libc::killpg(pid, signal_number);
        } else {
            libc::kill(pid, signal_number);
        }
    }
}

/// The tool runs below Steam's launch wrappers (`steam-runtime-supervisor`,
/// `reaper`, `steam-launch-wrapper`). They keep the full `<game.exe>` path in
/// their argv for the whole session, so only excluding the process and its
/// direct parent makes `wait_for_game_exit` wait for its own ancestor.
fn ancestor_pids() -> HashSet<i32> {
    let mut ancestors = HashSet::new();
    let mut pid = unsafe { libc::getppid() };
    for _ in 0..64 {
        if pid <= 0 || !ancestors.insert(pid) {
            break;
        }
        let Ok(text) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            break;
        };
        let Some((_, rest)) = text.rsplit_once(')') else {
            break;
        };
        let Some(parent) = rest
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<i32>().ok())
        else {
            break;
        };
        pid = parent;
    }
    ancestors
}

fn args_of(cmdline: &[u8]) -> Vec<String> {
    cmdline
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

/// Does this command line belong to `needle` (and not to one of the wrappers)?
/// `for_game` additionally drops Windows launchers that take the game path as
/// an argument (`TruckersMP-Launcher.exe --game <game>.exe`).
fn cmdline_matches(args: &[String], needle: &str, for_game: bool) -> bool {
    if args.is_empty() || needle.is_empty() {
        return false;
    }
    let needle = needle.to_ascii_lowercase();
    if !args
        .iter()
        .any(|arg| arg.to_ascii_lowercase().contains(&needle))
    {
        return false;
    }
    let joined = args.join(" ").to_ascii_lowercase();
    if WRAPPERS.iter().any(|word| joined.contains(word)) {
        return false;
    }
    if for_game && !needle.contains("truckersmp") && joined.contains("truckersmp-launch") {
        return false;
    }
    // `python3 …/proton run <exe>` carries the target path from the first
    // moment; the real program is the child, not this wrapper.
    if args.iter().any(|arg| arg == "proton" || arg.ends_with("/proton")) {
        return false;
    }
    true
}

fn process_running(needle: &str, for_game: bool) -> bool {
    if needle.is_empty() {
        return false;
    }
    let own = std::process::id() as i32;
    let ancestors = ancestor_pids();
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        if pid == own || ancestors.contains(&pid) {
            continue;
        }
        let Ok(cmdline) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if cmdline_matches(&args_of(&cmdline), needle, for_game) {
            return true;
        }
    }
    false
}

enum GameWait {
    Found,
    Skipped,
    Timeout,
    Stopped,
    Exited(ExitStatus),
}

enum Gone {
    Yes,
    Stopped,
    TimedOut,
}

fn find_game(
    control: &Control,
    child: &mut Child,
    needle: &str,
    limit: Limit,
) -> Result<GameWait, String> {
    if limit.none() {
        return Ok(GameWait::Skipped);
    }
    let deadline = limit.deadline();
    loop {
        if control.stop.load(Ordering::SeqCst) {
            return Ok(GameWait::Stopped);
        }
        if process_running(needle, false) {
            return Ok(GameWait::Found);
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(GameWait::Exited(status));
        }
        if let Some(deadline) = deadline {
            if Instant::now() >= deadline {
                return Ok(GameWait::Timeout);
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn wait_for_appear(control: &Control, exe: &str, limit: Limit) -> bool {
    let needle = base_name(exe);
    let deadline = limit.deadline();
    loop {
        if control.stop.load(Ordering::SeqCst) {
            return false;
        }
        if process_running(&needle, true) {
            return true;
        }
        if let Some(deadline) = deadline {
            if Instant::now() >= deadline {
                return false;
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn wait_until_gone(control: &Control, needle: &str, limit: Limit) -> Gone {
    let deadline = limit.deadline();
    loop {
        if !process_running(needle, true) {
            return Gone::Yes;
        }
        if control.stop.load(Ordering::SeqCst) {
            return Gone::Stopped;
        }
        if let Some(deadline) = deadline {
            if Instant::now() >= deadline {
                return Gone::TimedOut;
            }
        }
        thread::sleep(Duration::from_millis(1000));
    }
}

/// The app's own bundled library directory (bundle: `usr/bin` ->
/// `usr/lib`). A source install has no such layout.
fn bundle_lib() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?;
    if bin.file_name() != Some(std::ffi::OsStr::new("bin")) {
        return None;
    }
    let lib = bin.parent()?.join("lib");
    lib.is_dir().then_some(lib)
}

/// Proton must not inherit the bundle's library or Qt plugin paths: they
/// would shadow its own libs. Other `LD_LIBRARY_PATH` entries (Steam's) stay.
pub fn clean_proton_env(command: &mut Command) {
    if let Some(bundle) = bundle_lib() {
        if let Ok(paths) = std::env::var("LD_LIBRARY_PATH") {
            let kept: Vec<&str> = paths
                .split(':')
                .filter(|path| !path.is_empty() && !Path::new(path).starts_with(&bundle))
                .collect();
            if kept.is_empty() {
                command.env_remove("LD_LIBRARY_PATH");
            } else {
                command.env("LD_LIBRARY_PATH", kept.join(":"));
            }
        }
    }
    command.env_remove("QT_PLUGIN_PATH");
    command.env_remove("QT_QPA_PLATFORM_PLUGIN_PATH");
}

fn spawn_proton(
    spec: &LaunchSpec,
    arg: &str,
    exe: &str,
    log_file: &mut fs::File,
) -> std::io::Result<Child> {
    let mut command = Command::new(&spec.proton);
    clean_proton_env(&mut command);
    command.arg(arg).arg(exe);
    command.env("STEAM_COMPAT_DATA_PATH", &spec.compatdata);
    if let Some(root) = &spec.steam_root {
        command.env("STEAM_COMPAT_CLIENT_INSTALL_PATH", root);
        // Proton's `gamedrive` compat option (proton: try_get_game_library_dir)
        // keeps `dosdevices/s:` only when it gets both the library list and the
        // game directory; otherwise it *deletes* S:.  TruckersMP stores the
        // game path as `S:\steamapps\common\...`, so without these two the
        // launcher says "installed:false" and PLAY cannot find the game.
        let game_dir = crate::games::game_dir(root, &spec.appid);
        let install_path = game_dir.clone().unwrap_or_else(|| {
            if spec.game_exe.is_empty() {
                PathBuf::new()
            } else {
                PathBuf::from(&spec.game_exe)
            }
        });
        if !install_path.as_os_str().is_empty() {
            let library = install_path
                .ancestors()
                .find(|dir| dir.ends_with("steamapps"))
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.join("steamapps"));
            command.env("STEAM_COMPAT_LIBRARY_PATHS", library);
            command.env("STEAM_COMPAT_INSTALL_PATH", install_path);
        }
    }
    command.env("SteamAppId", &spec.appid);
    command.env("SteamGameId", &spec.appid);
    command.env("SteamOverlayGameId", &spec.appid);
    command.stdout(Stdio::from(log_file.try_clone()?));
    command.stderr(Stdio::from(log_file.try_clone()?));
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    command.spawn()
}

fn start_sidecars(spec: &LaunchSpec, control: &Control, file: &mut fs::File, wait_each: bool) {
    let delay = Duration::from_secs(spec.extra_delay);
    for extra in &spec.extras {
        if control.stop.load(Ordering::SeqCst) {
            break;
        }
        if !Path::new(extra).is_file() {
            set(control, |state| state.extras_skipped.push(extra.clone()));
            log(file, "extra missing", extra);
            continue;
        }
        thread::sleep(delay);
        match spawn_proton(spec, "runinprefix", extra, file) {
            Ok(mut sidecar) => {
                let pid = sidecar.id() as i32;
                if let Ok(mut children) = control.children.lock() {
                    children.push(pid);
                }
                set(control, |state| state.extras_started.push(extra.clone()));
                log(file, "extra started", &format!("{extra} (pid {pid})"));
                thread::spawn(move || {
                    let _ = sidecar.wait();
                });
                // A game-direct launch starts every sidecar before the game, so
                // a second Wine session in the same prefix must not boot while
                // the previous one is still coming up.
                if wait_each && spec.extra_timeout > 0 {
                    let limit = Limit::start(spec.extra_timeout);
                    let up = wait_for_appear(control, extra, limit);
                    log(
                        file,
                        "extra wait",
                        if up { "up" } else { "did not appear" },
                    );
                }
            }
            Err(err) => {
                set(control, |state| state.extras_skipped.push(extra.clone()));
                log(file, "extra", &format!("{extra} could not start: {err}"));
            }
        }
    }
}

fn stop_sidecars(control: &Control, file: &mut fs::File) {
    let sidecars = control
        .children
        .lock()
        .map(|children| children.clone())
        .unwrap_or_default();
    if sidecars.is_empty() {
        return;
    }
    log(file, "extras stopping", &format!("{}", sidecars.len()));
    for pid in &sidecars {
        signal(*pid, libc::SIGTERM);
    }
    // Wine GUIs often ignore the first signal while pumping messages.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && sidecars.iter().any(|pid| pid_alive(*pid)) {
        thread::sleep(Duration::from_millis(100));
    }
    for pid in &sidecars {
        if pid_alive(*pid) {
            signal(*pid, libc::SIGKILL);
        }
    }
    log(file, "extras stopped", &format!("{}", sidecars.len()));
}

fn finish_state(control: &Control, rc: i32, error: String) {
    set(control, |state| {
        state.done = true;
        state.rc = rc;
        state.error = error;
        state.stopping = false;
    });
    if let Ok(mut done) = control.done_at.lock() {
        *done = Some(SystemTime::now());
    }
}

fn run(spec: Arc<LaunchSpec>, control: Control) {
    let Ok(path) = control
        .state
        .lock()
        .map(|state| PathBuf::from(&state.log))
    else {
        return;
    };
    let mut file = match open_log(&path) {
        Ok(file) => file,
        Err(err) => {
            finish_state(
                &control,
                1,
                format!("could not open the log {}: {err}", path.display()),
            );
            return;
        }
    };

    let (rc, error) = match run_session(&spec, &control, &mut file) {
        Ok(rc) => (rc, String::new()),
        Err(error) => {
            log_line(&mut file, &format!("ERROR: {error}"));
            (1, error)
        }
    };
    // Both paths end the log with the marker the tray (and the old session
    // window) look for: a failed launch must not leave the tray waiting.
    log_line(&mut file, &format!("=== steam-mp-launch done (rc={rc}) ==="));
    finish_state(&control, rc, error);
}

fn run_session(spec: &LaunchSpec, control: &Control, file: &mut fs::File) -> Result<i32, String> {
    if let Ok(state) = control.state.lock() {
        log_line(file, "=== steam-mp-launch ===");
        log(file, "appid", &state.appid);
        log(file, "game", &state.game);
        log(file, "proton", &state.proton);
        log(file, "launcher (chosen)", &state.exe);
        log(file, "compatdata", &spec.compatdata.display().to_string());
        log(
            file,
            "wait for game",
            &spec
                .wait_limit
                .map(|secs| Limit::keep(secs).label())
                .unwrap_or_else(|| "no (WAIT_FOR_GAME=0)".to_owned()),
        );
        log(file, "sidecars", if spec.sidecars { "on" } else { "off (picker)" });
        log(file, "sidecar wait", &Limit::start(spec.extra_timeout).label());
    }

    let direct_game = !spec.game_exe.is_empty() && base_name(&spec.game_exe) == base_name(&spec.exe);
    // Game-direct mode: sidecars go first, or a client like TrucksBook misses
    // the boot of the game.
    if direct_game && !spec.extras.is_empty() {
        start_sidecars(spec, control, file, true);
    }

    let outcome = (|| -> Result<i32, String> {
        let mut child = spawn_proton(spec, "run", &spec.exe, file)
            .map_err(|err| format!("could not start Proton: {err}"))?;
        let pid = child.id() as i32;
        if let Ok(mut pgid) = control.pgid.lock() {
            *pgid = Some(pid);
        }
        set(control, |state| state.started = true);
        log(file, "session up", &format!("pid {pid}"));

        // Wait for the launcher (or the game) to appear; EXTRA_TIMEOUT bounds it.
        let waited = find_game(
            control,
            &mut child,
            &base_name(&spec.exe),
            Limit::start(spec.extra_timeout),
        )?;
        let mut status = None;
        let mut start_now = !direct_game;
        match waited {
            GameWait::Found => {
                set(control, |state| state.game_up = true);
                log(file, "game up", &spec.exe);
            }
            GameWait::Skipped => {}
            GameWait::Timeout => log(
                file,
                "sidecar wait",
                &format!("timed out after {}s", spec.extra_timeout),
            ),
            GameWait::Stopped => start_now = false,
            GameWait::Exited(exit) => {
                status = Some(exit);
                start_now = false;
            }
        }
        if start_now && !spec.extras.is_empty() {
            start_sidecars(spec, control, file, false);
        }

        let status = match status {
            Some(status) => status,
            None => child.wait().map_err(|err| format!("wait failed: {err}"))?,
        };

        // A launcher often hands over to the real game and exits first. In that
        // case keep the session (and its sidecars) alive while the game runs.
        if let Some(wait_secs) = spec.wait_limit {
            let game = base_name(&spec.game_exe);
            if !spec.game_exe.is_empty() && game != base_name(&spec.exe) {
                let running = process_running(&game, true)
                    || wait_for_appear(control, &spec.game_exe, Limit::Secs(HANDOFF_GRACE));
                if running {
                    set(control, |state| state.game_up = true);
                    log(file, "game up", &spec.game_exe);
                    match wait_until_gone(control, &game, Limit::keep(wait_secs)) {
                        Gone::Yes => log(file, "game gone", &game),
                        Gone::Stopped => log(file, "game gone", "stop requested"),
                        Gone::TimedOut => log(
                            file,
                            "game wait",
                            &format!("timed out after {wait_secs}s, ending the session"),
                        ),
                    }
                } else {
                    log(
                        file,
                        "game wait",
                        &format!("{} did not appear after {HANDOFF_GRACE}s", spec.game_exe),
                    );
                }
            }
        }

        Ok(status.code().unwrap_or(1))
    })();

    // Always clean up: in game-direct mode the sidecars run before the game, so
    // a failed main spawn still leaves Wine sessions in the prefix.
    stop_sidecars(control, file);
    // Do not leave a wineserver/xalia/Electron process behind in the prefix,
    // or its single-instance mutex stops the next launch from opening.
    let pgid = control.pgid.lock().ok().and_then(|pgid| *pgid);
    if let Some(pgid) = pgid {
        signal(pgid, libc::SIGTERM);
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline && pid_alive(pgid) {
            thread::sleep(Duration::from_millis(50));
        }
        signal(pgid, libc::SIGKILL);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn zero_means_off_for_extra_timeout_and_no_limit_for_the_game() {
        assert_eq!(Limit::start(0), Limit::None);
        assert_eq!(Limit::start(120), Limit::Secs(120));
        assert_eq!(Limit::keep(0), Limit::Forever);
        assert_eq!(Limit::keep(600), Limit::Secs(600));
        assert!(Limit::None.deadline().is_none());
        assert!(Limit::Forever.deadline().is_none());
        assert!(Limit::Secs(1).deadline().is_some());
        assert_eq!(Limit::start(0).label(), "no wait");
        assert_eq!(Limit::keep(0).label(), "no limit");
    }

    #[test]
    fn wrappers_never_count_as_the_game() {
        let game = "eurotrucks2.exe";
        // Proton's own wrapper carries the target path from the start.
        assert!(!cmdline_matches(
            &args(&[
                "python3",
                "/home/me/.local/share/Steam/steamapps/common/Proton - Experimental/proton",
                "run",
                "/games/Euro Truck Simulator 2/bin/win_x64/eurotrucks2.exe",
            ]),
            game,
            false
        ));
        // Steam's runtime wrappers are the tool's ancestors and stay alive.
        assert!(!cmdline_matches(
            &args(&[
                "/home/me/.local/share/Steam/steamrt64/reaper",
                "SteamLaunch",
                "AppId=227300",
                "--",
                "/tools/steam-mp-launcher-next/steam-mp-launch",
                "waitforexitandrun",
                "/games/Euro Truck Simulator 2/bin/win_x64/eurotrucks2.exe",
            ]),
            game,
            false
        ));
        assert!(!cmdline_matches(
            &args(&["/opt/steam-mp-launcher", "--tray", "--label", "TruckersMP"]),
            "truckersmp",
            false
        ));
        // The real program (and the real game) still match.
        assert!(cmdline_matches(
            &args(&["/games/Euro Truck Simulator 2/bin/win_x64/eurotrucks2.exe"]),
            game,
            false
        ));
        assert!(cmdline_matches(
            &args(&[
                "/prefix/drive_c/Program Files (x86)/TrucksBook Client/TB Client.exe",
            ]),
            "tb client.exe",
            false
        ));
    }

    #[test]
    fn a_launcher_that_mentions_the_game_is_not_the_game() {
        let truckersmp = args(&[
            "Z:/prefix/drive_c/users/steamuser/AppData/Local/TruckersMP/TruckersMP-Launcher.exe",
            "--game",
            "/games/Euro Truck Simulator 2/bin/win_x64/eurotrucks2.exe",
        ]);
        assert!(!cmdline_matches(&truckersmp, "eurotrucks2.exe", true));
        assert!(cmdline_matches(&truckersmp, "truckersmp-launcher.exe", false));
        // The same process is still a valid "session up" target.
        assert!(cmdline_matches(&truckersmp, "truckersmp-launcher.exe", true));
    }

    #[test]
    fn unrelated_processes_do_not_match() {
        let game = "eurotrucks2.exe";
        assert!(!cmdline_matches(&args(&["/usr/bin/firefox", "about:blank"]), game, false));
        assert!(!cmdline_matches(&args(&[]), game, false));
        assert!(!cmdline_matches(&args(&["/games/eurotrucks2.exe"]), "", false));
    }

    #[test]
    fn only_the_newest_finished_sessions_are_kept() {
        let sessions = Sessions::new();
        {
            let mut map = sessions.map.lock().unwrap();
            for id in 1..=12u64 {
                let state = SessionState {
                    id,
                    appid: "227300".to_owned(),
                    game: "Game".to_owned(),
                    exe: "/games/launcher.exe".to_owned(),
                    proton: "/proton".to_owned(),
                    log: String::new(),
                    started: false,
                    game_up: false,
                    extras_started: Vec::new(),
                    extras_skipped: Vec::new(),
                    stopping: false,
                    done: true,
                    rc: 0,
                    error: String::new(),
                };
                map.insert(
                    id,
                    Control {
                        state: Arc::new(Mutex::new(state)),
                        stop: Arc::new(AtomicBool::new(false)),
                        pgid: Arc::new(Mutex::new(None)),
                        children: Arc::new(Mutex::new(Vec::new())),
                        done_at: Arc::new(Mutex::new(Some(
                            SystemTime::UNIX_EPOCH + Duration::from_secs(id),
                        ))),
                    },
                );
            }
        }
        sessions.prune();
        let map = sessions.map.lock().unwrap();
        assert_eq!(map.len(), 8);
        assert!(map.contains_key(&12));
        assert!(!map.contains_key(&1));
    }
}
