//! JSON command dispatch shared by the C ABI and the debug CLI.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;

use serde_json::{json, Value};

use crate::config::Config;
use crate::games::{self, Game};
use crate::icons;
use crate::launch::{LaunchSpec, Sessions};
use crate::lists;
use crate::logs;
use crate::paths::Paths;
use crate::protons;
use crate::saved;
use crate::settings;
use crate::steam_art;

pub struct Ctx {
    pub paths: Paths,
    pub config: Config,
    pub root: Option<PathBuf>,
    pub sessions: Sessions,
}

impl Default for Ctx {
    fn default() -> Self {
        Self::new()
    }
}

impl Ctx {
    pub fn new() -> Self {
        let paths = Paths::detect();
        let config = Config::load(&paths.conf);
        let root = protons::steam_root();
        Self {
            paths,
            config,
            root,
            sessions: Sessions::new(),
        }
    }

    fn save_choice(&self, appid: &str, value: &str) -> Result<(), String> {
        let target = self
            .paths
            .game_saved(appid)
            .unwrap_or_else(|| self.paths.saved.clone());
        saved::save_choice(&target, value)
    }

    fn game(&self, appid: &str, requested_exe: &str) -> Game {
        let conf_dir = self.paths.conf_dir.clone();
        if let Some(game) = games::games_using_tool(self.root.as_deref(), &conf_dir)
            .into_iter()
            .find(|game| game.appid == appid)
        {
            if !requested_exe.is_empty() {
                return Game {
                    exe: requested_exe.to_owned(),
                    ..game
                };
            }
            return game;
        }
        let name = self
            .root
            .as_deref()
            .map(|root| games::game_name(root, appid))
            .unwrap_or_else(|| format!("app{appid}"));
        let exe = if !requested_exe.is_empty() {
            requested_exe.to_owned()
        } else {
            self.root
                .as_deref()
                .map(|root| games::find_exe(root, appid))
                .unwrap_or_default()
        };
        let compatdata = self
            .root
            .as_deref()
            .and_then(|root| games::prefix_for(root, appid))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        Game {
            appid: appid.to_owned(),
            name,
            prefix: String::new(),
            source: "run".to_owned(),
            exe,
            compatdata,
        }
    }

    fn compatdata(&self, game: &Game, request: &Value) -> PathBuf {
        let requested = request["compatdata"].as_str().unwrap_or("");
        if !requested.is_empty() {
            return PathBuf::from(requested);
        }
        if let Ok(env) = std::env::var("STEAM_COMPAT_DATA_PATH") {
            if !env.is_empty() {
                return PathBuf::from(env);
            }
        }
        if !game.compatdata.is_empty() {
            return PathBuf::from(&game.compatdata);
        }
        if let Some(root) = &self.root {
            return root.join("steamapps/compatdata").join(&game.appid);
        }
        PathBuf::new()
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn number(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn error(message: impl Into<String>) -> Value {
    json!({ "ok": false, "error": message.into() })
}

/// The picker can turn the sidecars off for one launch; absent means on.
fn sidecars_enabled(request: &Value) -> bool {
    request
        .get("sidecars")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

/// `WAIT_FOR_GAME=0` disables the hold completely; `WAIT_FOR_GAME_TIMEOUT=0`
/// means "no limit" (see `docs/contracts.md`).
fn wait_limit_from(wait_for_game: &str, timeout: i64) -> Option<u64> {
    if wait_for_game != "1" {
        return None;
    }
    Some(timeout.max(0) as u64)
}

pub fn handle(ctx: &mut Ctx, request: &Value) -> Value {
    // The conf file can change outside this window; re-read it so an edit is
    // picked up without a restart.
    ctx.config.refresh();
    let cmd = text(request, "cmd");
    let result = match cmd.as_str() {
        "picker" => picker(ctx, request),
        "manager" => manager(ctx, request),
        "launch" => launch(ctx, request),
        "run_extra" => run_extra(ctx, request),
        "session" => session(ctx, request),
        "stop_session" => stop_session(ctx, request),
        "icon" => icon_cmd(request),
        "list_add" => list_add(ctx, request),
        "list_remove" => list_remove(ctx, request),
        "list_relink" => list_relink(ctx, request),
        "list_rename" => list_rename(ctx, request),
        "list_scope" => list_scope(ctx, request),
        "forget" => forget(ctx, request),
        "save_choice" => save_choice(ctx, request),
        "protons" => protons_cmd(ctx),
        "set_proton" => set_proton(ctx, request),
        "settings" => settings_cmd(ctx),
        "set_setting" => set_setting(ctx, request),
        "restore_settings" => restore_settings(ctx, request),
        "logs" => logs_cmd(ctx),
        "read_log" => read_log(ctx, request),
        "clear_logs" => clear_logs(ctx, request),
        "games" => games_cmd(ctx),
        "install_find_installer" => Ok(crate::install::find_installer(&text(request, "kind"))),
        "install_detect" => install_detect(ctx, request),
        "install_prepare" => crate::install::prepare(&text(request, "path")),
        "install_probe" => install_probe(ctx, request),
        "install_close" => install_close(ctx, request),
        "install_finish" => install_finish(ctx, request),
        other => Err(format!("unknown command: {other}")),
    };
    match result {
        Ok(mut value) => {
            if let Some(object) = value.as_object_mut() {
                object.insert("ok".to_owned(), Value::Bool(true));
            }
            value
        }
        Err(message) => error(message),
    }
}

type CmdResult = Result<Value, String>;

fn appid(ctx: &Ctx, request: &Value) -> String {
    games::resolve_appid(
        &text(request, "appid"),
        ctx.root.as_deref(),
        &ctx.paths.conf_dir,
    )
}

fn settings_map(ctx: &Ctx) -> Value {
    let mut map = serde_json::Map::new();
    for (key, value) in settings::current(&ctx.config) {
        map.insert(key.to_owned(), Value::String(value));
    }
    Value::Object(map)
}

fn entries_json(path: &Path, saved_path: &str) -> Value {
    let entries = lists::read_entries(path);
    let rows: Vec<Value> = entries
        .into_iter()
        .map(|entry| {
            json!({
                "label": entry.label,
                "path": entry.path,
                "icon": icons::icon_for(&entry.path),
                "missing": !Path::new(&entry.path).is_file(),
                "remembered": entry.path == saved_path,
            })
        })
        .collect();
    json!(rows)
}

fn extras_json(path: &Path) -> Value {
    let entries = lists::read_entries(path);
    let rows: Vec<Value> = entries
        .into_iter()
        .map(|entry| {
            json!({
                "label": entry.label,
                "path": entry.path,
                "icon": icons::icon_for(&entry.path),
                "missing": !Path::new(&entry.path).is_file(),
            })
        })
        .collect();
    json!(rows)
}

fn icon_cmd(request: &Value) -> CmdResult {
    let path = text(request, "path");
    Ok(json!({ "icon": icons::icon_for(&path) }))
}

fn picker(ctx: &mut Ctx, request: &Value) -> CmdResult {
    let appid = appid(ctx, request);
    let requested_exe = text(request, "game_exe");
    let game = ctx.game(&appid, &requested_exe);
    let compatdata = ctx.compatdata(&game, request);
    let prefix_conf = compatdata.join("config_info");
    let proton = protons::resolve(
        &ctx.config,
        ctx.root.as_deref(),
        prefix_conf.is_file().then_some(prefix_conf.as_path()),
    );
    let launchers_file = ctx.paths.launchers_file(&appid);
    let extras_file = ctx.paths.extras_file(&appid);
    let game_saved = ctx.paths.game_saved(&appid);
    let saved_path = saved::read_saved(game_saved.as_deref(), &ctx.paths.saved);
    let entries = entries_json(&launchers_file, &saved_path);
    let extras = extras_json(&extras_file);
    let proton_name = proton
        .as_deref()
        .map(protons::proton_name)
        .unwrap_or_default();
    // Hiding the installer button needs to know what is already in the prefix.
    let prefix = protons::prefix_of(&compatdata);
    let (truckersmp_installed, trucksbook_installed) = if compatdata.as_os_str().is_empty() {
        (false, false)
    } else {
        (
            !crate::install::find_truckersmp_launcher(&prefix).is_empty(),
            !crate::install::find_trucksbook_client(&prefix).is_empty(),
        )
    };
    Ok(json!({
        "game": {
            "appid": appid,
            "name": game.name,
            "exe": game.exe,
            "compatdata": compatdata.to_string_lossy(),
            "prefix": protons::prefix_of(&compatdata).to_string_lossy(),
            "proton": proton.as_deref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            "proton_name": proton_name,
            "banner": steam_art::find_banner(&appid),
            "icon": steam_art::find_icon(&appid),
        },
        "entries": entries,
        "extras": extras,
        "saved": saved_path,
        "launchers_file": launchers_file.to_string_lossy(),
        "extras_file": extras_file.to_string_lossy(),
        "logs_dir": ctx.paths.logs.to_string_lossy(),
        "installers": {
            "truckersmp": truckersmp_installed,
            "trucksbook": trucksbook_installed,
        },
        "cancel_mode": ctx.config.get("GUI_CANCEL", "abort"),
        "confirm_delay": ctx.config.get_int("GUI_CONFIRM_DELAY", 3),
        "launcher_gui": settings::normalize("LAUNCHER_GUI", &ctx.config.get("LAUNCHER_GUI", "")),
        "fallback_to_game": settings::normalize("FALLBACK_TO_GAME", &ctx.config.get("FALLBACK_TO_GAME", "")),
    }))
}

fn manager(ctx: &mut Ctx, request: &Value) -> CmdResult {
    let appid = appid(ctx, request);
    let requested_exe = text(request, "game_exe");
    let game = ctx.game(&appid, &requested_exe);
    let compatdata = ctx.compatdata(&game, request);
    let prefix_conf = compatdata.join("config_info");
    let proton = protons::resolve(
        &ctx.config,
        ctx.root.as_deref(),
        prefix_conf.is_file().then_some(prefix_conf.as_path()),
    );
    let saved_path = saved::read_saved(ctx.paths.game_saved(&appid).as_deref(), &ctx.paths.saved);
    let entries = entries_json(&ctx.paths.launchers_file(&appid), &saved_path);
    let extras = extras_json(&ctx.paths.extras_file(&appid));
    let games = games::games_using_tool(ctx.root.as_deref(), &ctx.paths.conf_dir);
    let logs = logs::list_runs(&ctx.paths.logs, 20);

    let mut checks = Vec::new();
    checks.push(json!({
        "state": if proton.is_some() { "ok" } else { "error" },
        "title": if proton.is_some() { "Proton found" } else { "No Proton found" },
        "detail": proton.as_deref().map(protons::proton_name).unwrap_or_default(),
        "view": "proton",
    }));
    checks.push(json!({
        "state": if !game.exe.is_empty() && Path::new(&game.exe).is_file() { "ok" } else { "warn" },
        "title": "Game executable",
        "detail": if game.exe.is_empty() { "run the game through Steam once".to_owned() } else { game.exe.clone() },
        "view": "overview",
    }));
    checks.push(json!({
        "state": if compatdata.is_dir() { "ok" } else { "warn" },
        "title": "Wine prefix",
        "detail": compatdata.to_string_lossy(),
        "view": "overview",
    }));
    checks.push(json!({
        "state": if entries.as_array().is_some_and(|a| !a.is_empty()) { "ok" } else { "warn" },
        "title": "Launchers",
        "detail": format!("{} configured", entries.as_array().map(Vec::len).unwrap_or(0)),
        "view": "launchers",
    }));

    let launch_cfg = json!({
        "wait_for_game": ctx.config.get("WAIT_FOR_GAME", "1"),
        "wait_limit": ctx.config.get_int("WAIT_FOR_GAME_TIMEOUT", 600),
        "extra_timeout": ctx.config.get_int("EXTRA_TIMEOUT", 120),
        "extra_delay": ctx.config.get_int("EXTRA_DELAY", 5),
    });

    Ok(json!({
        "game": {
            "appid": appid, "name": game.name, "exe": game.exe,
            "compatdata": compatdata.to_string_lossy(),
            "prefix": protons::prefix_of(&compatdata).to_string_lossy(),
            "proton": proton.as_deref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            "proton_name": proton.as_deref().map(protons::proton_name).unwrap_or_default(),
            "banner": steam_art::find_banner(&appid),
            "icon": steam_art::find_icon(&appid),
        },
        "games": games,
        "entries": entries,
        "extras": extras,
        "extras_file": ctx.paths.extras_file(&appid).to_string_lossy(),
        "launchers_file": ctx.paths.launchers_file(&appid).to_string_lossy(),
        "logs_dir": ctx.paths.logs.to_string_lossy(),
        "saved": saved_path,
        "scope_launchers": lists::scope(&ctx.paths.launchers, "launchers", &appid),
        "scope_extras": lists::scope(&ctx.paths.extras, "extras", &appid),
        "readiness": checks,
        "logs": logs.iter().map(|run| json!({
            "path": run.path, "name": run.name, "appid": run.appid,
            "title": if !run.launcher.is_empty() { run.launcher.clone() } else if !run.game.is_empty() { run.game.clone() } else { run.name.clone() },
            "time": logs::time_label(run.mtime), "status": run.status,
        })).collect::<Vec<_>>(),
        "settings": settings_map(ctx),
        "launch": launch_cfg,
    }))
}

fn launch(ctx: &mut Ctx, request: &Value) -> CmdResult {
    let appid = appid(ctx, request);
    let game = ctx.game(&appid, &text(request, "game_exe"));
    let requested = text(request, "exe");
    let chosen = if requested.is_empty() || requested == saved::GAME_SENTINEL {
        game.exe.clone()
    } else {
        requested.clone()
    };
    if chosen.is_empty() {
        return Err("no program to launch".to_owned());
    }
    let compatdata = ctx.compatdata(&game, request);
    let prefix_conf = compatdata.join("config_info");
    let proton = protons::resolve(
        &ctx.config,
        ctx.root.as_deref(),
        prefix_conf.is_file().then_some(prefix_conf.as_path()),
    )
    .ok_or("no Proton installation found")?;
    let launch_delay = ctx.config.get_int("EXTRA_DELAY", 5).max(0) as u64;
    let extra_timeout = ctx.config.get_int("EXTRA_TIMEOUT", 120).max(0) as u64;
    let wait_limit = wait_limit_from(
        &ctx.config.get("WAIT_FOR_GAME", "1"),
        ctx.config.get_int("WAIT_FOR_GAME_TIMEOUT", 600),
    );
    let sidecars = sidecars_enabled(request);
    let extras: Vec<String> = if sidecars {
        lists::read_entries(&ctx.paths.extras_file(&appid))
            .into_iter()
            .map(|entry| entry.path)
            .collect()
    } else {
        Vec::new()
    };
    if !requested.is_empty() && requested != saved::GAME_SENTINEL && flag(request, "remember") {
        if let Err(err) = ctx.save_choice(&appid, &requested) {
            return Err(format!("could not remember the choice: {err}"));
        }
    }
    if compatdata.as_os_str().is_empty() {
        return Err("no compatdata/prefix for this game".to_owned());
    }
    let spec = LaunchSpec {
        appid,
        game: game.name,
        exe: chosen,
        game_exe: game.exe,
        proton,
        compatdata,
        steam_root: ctx.root.clone(),
        extras,
        sidecars,
        extra_timeout,
        wait_limit,
        extra_delay: launch_delay,
        logs_dir: ctx.paths.logs.clone(),
    };
    let (id, state) = ctx.sessions.start(spec)?;
    Ok(json!({ "session": state, "id": id }))
}

/// Start an additional Windows program through Proton in this game's prefix
/// (used by the tray menu).  The process is detached and not tracked.
fn run_extra(ctx: &Ctx, request: &Value) -> CmdResult {
    let exe = text(request, "exe");
    if exe.is_empty() || !Path::new(&exe).is_file() {
        return Err("program not found".to_owned());
    }
    let appid = appid(ctx, request);
    let game = ctx.game(&appid, "");
    let compatdata = ctx.compatdata(&game, request);
    let prefix_conf = compatdata.join("config_info");
    let proton = protons::resolve(
        &ctx.config,
        ctx.root.as_deref(),
        prefix_conf.is_file().then_some(prefix_conf.as_path()),
    )
    .ok_or("no Proton installation found")?;

    let mut command = std::process::Command::new(proton);
    command.arg("runinprefix").arg(&exe);
    crate::launch::clean_proton_env(&mut command);
    command.env("STEAM_COMPAT_DATA_PATH", &compatdata);
    if let Some(root) = &ctx.root {
        command.env("STEAM_COMPAT_CLIENT_INSTALL_PATH", root);
    }
    command.env("SteamAppId", &appid);
    command.env("SteamGameId", &appid);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let child = command
        .spawn()
        .map_err(|err| format!("could not start {exe}: {err}"))?;
    let pid = child.id();
    // The UI process outlives the program; without a reaper every "Run once"
    // would leave a zombie behind.
    thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(json!({ "pid": pid, "path": exe }))
}

fn session(ctx: &Ctx, request: &Value) -> CmdResult {
    let id = number(request, "id").ok_or("missing session id")?;
    ctx.sessions
        .state(id)
        .map(|state| json!(state))
        .ok_or_else(|| "unknown session".to_owned())
}

fn stop_session(ctx: &Ctx, request: &Value) -> CmdResult {
    let id = number(request, "id").ok_or("missing session id")?;
    ctx.sessions.stop(id)?;
    Ok(json!({}))
}

fn target_file(ctx: &Ctx, request: &Value) -> Result<PathBuf, String> {
    let which = text(request, "which");
    let appid = appid(ctx, request);
    let shared = ctx.paths.shared(&which).to_path_buf();
    if text(request, "scope") == "shared" {
        return Ok(shared);
    }
    Ok(Paths::effective_named(&shared, &which, &appid))
}

fn list_add(ctx: &Ctx, request: &Value) -> CmdResult {
    let path = target_file(ctx, request)?;
    let label = if text(request, "label").is_empty() {
        lists::default_label(&text(request, "path"))
    } else {
        text(request, "label")
    };
    lists::add(&path, &label, &text(request, "path"))?;
    Ok(json!({ "path_file": path.to_string_lossy() }))
}

fn list_remove(ctx: &Ctx, request: &Value) -> CmdResult {
    let path = target_file(ctx, request)?;
    let removed = lists::remove(&path, &text(request, "path"))?;
    Ok(json!({ "removed": removed }))
}

fn list_relink(ctx: &Ctx, request: &Value) -> CmdResult {
    let path = target_file(ctx, request)?;
    let changed = lists::relink(&path, &text(request, "old"), &text(request, "new"))?;
    Ok(json!({ "changed": changed }))
}

fn list_rename(ctx: &Ctx, request: &Value) -> CmdResult {
    let path = target_file(ctx, request)?;
    let changed = lists::rename(&path, &text(request, "old"), &text(request, "new"))?;
    Ok(json!({ "changed": changed }))
}

fn list_scope(ctx: &Ctx, request: &Value) -> CmdResult {
    let which = text(request, "which");
    let appid = appid(ctx, request);
    let shared = ctx.paths.shared(&which);
    let own = Paths::game_file_named(shared, &which, &appid)
        .ok_or("per-game lists need a Steam appid")?;
    lists::scope_action(shared, &own, &text(request, "scope"), flag(request, "copy"))?;
    Ok(json!({ "scope": lists::scope(shared, &which, &appid) }))
}

fn forget(ctx: &mut Ctx, request: &Value) -> CmdResult {
    let appid = appid(ctx, request);
    saved::forget(ctx.paths.game_saved(&appid).as_deref(), &ctx.paths.saved)?;
    Ok(json!({}))
}

fn save_choice(ctx: &mut Ctx, request: &Value) -> CmdResult {
    let appid = appid(ctx, request);
    let path = text(request, "path");
    if path.is_empty() {
        return Err("empty choice".to_owned());
    }
    ctx.save_choice(&appid, &path)?;
    Ok(json!({}))
}

fn protons_cmd(ctx: &Ctx) -> CmdResult {
    Ok(json!({
        "current": protons::resolve(&ctx.config, ctx.root.as_deref(), None)
            .map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        "items": protons::list_protons(ctx.root.as_deref()),
    }))
}

fn set_proton(ctx: &mut Ctx, request: &Value) -> CmdResult {
    let value = text(request, "path");
    if value.is_empty() {
        return Err("empty Proton path".to_owned());
    }
    ctx.config.set("PROTON", &value)?;
    Ok(json!({ "current": value }))
}

fn settings_cmd(ctx: &Ctx) -> CmdResult {
    Ok(json!({
        "groups": settings::GROUPS,
        "specs": settings::SPECS,
        "current": settings_map(ctx),
    }))
}

fn set_setting(ctx: &mut Ctx, request: &Value) -> CmdResult {
    let value = settings::set(
        &mut ctx.config,
        &text(request, "key"),
        &text(request, "value"),
    )?;
    Ok(json!({ "value": value }))
}

fn restore_settings(ctx: &mut Ctx, request: &Value) -> CmdResult {
    settings::restore(&mut ctx.config, &text(request, "group"))?;
    Ok(json!({}))
}

fn logs_cmd(ctx: &Ctx) -> CmdResult {
    let runs = logs::list_runs(&ctx.paths.logs, 0);
    Ok(json!(runs.iter().map(|run| json!({
        "path": run.path, "name": run.name, "appid": run.appid,
        "title": if !run.launcher.is_empty() { run.launcher.clone() } else if !run.game.is_empty() { run.game.clone() } else { run.name.clone() },
        "time": logs::time_label(run.mtime), "status": run.status,
    })).collect::<Vec<_>>()))
}

fn read_log(ctx: &Ctx, request: &Value) -> CmdResult {
    let path = PathBuf::from(text(request, "path"));
    if !logs::is_in_dir(&ctx.paths.logs, &path) {
        return Err("log path is outside the log directory".to_owned());
    }
    Ok(json!({ "text": logs::read_text(&path) }))
}

fn clear_logs(ctx: &Ctx, request: &Value) -> CmdResult {
    let days = number(request, "days").unwrap_or(7);
    let removed = logs::clear_older_than(&ctx.paths.logs, days);
    Ok(json!({ "removed": removed }))
}

fn install_compatdata(ctx: &Ctx, request: &Value) -> String {
    let game = ctx.game(&appid(ctx, request), "");
    let compatdata = if !text(request, "compatdata").is_empty() {
        PathBuf::from(text(request, "compatdata"))
    } else {
        ctx.compatdata(&game, request)
    };
    compatdata.to_string_lossy().into_owned()
}

fn install_detect(ctx: &Ctx, request: &Value) -> CmdResult {
    let appid = appid(ctx, request);
    Ok(crate::install::detect(
        ctx.root.as_deref(),
        &appid,
        &text(request, "kind"),
        &install_compatdata(ctx, request),
        &text(request, "game_exe"),
    ))
}

fn install_close(ctx: &Ctx, request: &Value) -> CmdResult {
    Ok(crate::install::close(
        &install_compatdata(ctx, request),
        ctx.root.as_deref(),
        &appid(ctx, request),
        request["since"].as_i64().unwrap_or(0),
        request["force"].as_bool().unwrap_or(false),
    ))
}

fn install_probe(ctx: &Ctx, request: &Value) -> CmdResult {
    Ok(crate::install::probe(
        ctx.root.as_deref(),
        &appid(ctx, request),
        &text(request, "kind"),
        &text(request, "game_exe"),
        &install_compatdata(ctx, request),
        request["since"].as_i64().unwrap_or(0),
    ))
}

fn install_finish(ctx: &Ctx, request: &Value) -> CmdResult {
    crate::install::finish(
        &ctx.paths,
        ctx.root.as_deref(),
        &appid(ctx, request),
        &text(request, "kind"),
        &text(request, "game_exe"),
        &install_compatdata(ctx, request),
        &text(request, "path"),
    )
}

fn games_cmd(ctx: &Ctx) -> CmdResult {
    Ok(json!({
        "items": games::games_using_tool(ctx.root.as_deref(), &ctx.paths.conf_dir),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sidecar_switch_defaults_to_on() {
        assert!(sidecars_enabled(&json!({})));
        assert!(sidecars_enabled(&json!({"sidecars": true})));
        assert!(!sidecars_enabled(&json!({"sidecars": false})));
    }

    #[test]
    fn game_wait_follows_the_documented_zero_meanings() {
        // WAIT_FOR_GAME=0: do not hold the session for the game.
        assert_eq!(wait_limit_from("0", 600), None);
        // WAIT_FOR_GAME_TIMEOUT=0: no limit (not "do not wait").
        assert_eq!(wait_limit_from("1", 0), Some(0));
        assert_eq!(wait_limit_from("1", 600), Some(600));
        assert_eq!(wait_limit_from("1", -3), Some(0));
    }
}
