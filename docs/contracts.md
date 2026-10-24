# Contracts — paths, files, API, logs

What the UI, the Steam runner and the Rust backend agree on. If one of these
changes, change the code and the tests in the same commit.

## Paths

| what | default | override |
|---|---|---|
| config dir | `$XDG_CONFIG_HOME/steam-mp-launcher` | — |
| settings | `<config>/steam-mp-launcher.conf` | `MP_CONF` |
| launchers | `<config>/launchers.list` | `MP_LIST` |
| sidecars | `<config>/extras.list` | `MP_EXTRAS` |
| remembered choice | `<config>/launcher` | `MP_SAVED` |
| per-game choice | `<config>/launcher-<appid>` | — |
| per-game lists | `<config>/launchers-<appid>.list`, `extras-<appid>.list` | — |
| disabled per-game list | `<file>.off` | — |
| run logs | `$XDG_STATE_HOME/steam-mp-launcher/logs` | — |
| icon cache | `$XDG_CACHE_HOME/steam-mp-launcher/icons` | — |
| single instance | `$XDG_RUNTIME_DIR/steam-mp-launcher-gui-<uid>.sock` | — |

`<appid>` is a positive decimal number; anything else means "no per-game
file". The legacy `TRUCKERSMP_*` variables are accepted when the `MP_*` one is
unset. `MP_SYSTEM_COMPAT_DIRS` (colon list, empty = off) adds distro
`compatibilitytools.d` directories, `MP_UI` points the Steam runner at the UI
binary.

## Settings

`steam-mp-launcher.conf` is `KEY=VALUE`, one per line; `#` starts a comment
and surrounding quotes are trimmed. Environment variables of the same name win
over the file. The file is re-read before every backend command, so an edit
made by hand (or by another tool) applies without a restart.

| key | default | meaning |
|---|---|---|
| `PROTON` | empty | a `proton` file or its directory; empty = pick automatically |
| `LAUNCHER_GUI` | `ask` | `ask` = always open the picker, `saved` = use the remembered choice |
| `GUI_CONFIRM_DELAY` | `3` | countdown before an automatic launch; `0` = no card |
| `GUI_CANCEL` | `abort` | cancel means `abort` (stop) or `game` (run the plain game) |
| `FALLBACK_TO_GAME` | `1` | no picker available and nothing remembered: `1` game, `0` nothing |
| `EXTRA_TIMEOUT` | `120` | seconds a program may take to appear; `0` = do not wait |
| `EXTRA_DELAY` | `5` | quiet gap between sidecar starts |
| `WAIT_FOR_GAME` | `1` | keep the session while the game runs |
| `WAIT_FOR_GAME_TIMEOUT` | `600` | limit for that wait; `0` = no limit |
| `UI_LANGUAGE` | empty | reserved for translation catalogs |

## Lists

`<label>|<path to .exe>`, one per line; blank lines and `#` comments are
ignored; a leading `~` expands to `$HOME`. Only `.exe` paths are accepted.
Writes are atomic (temp file + rename, mode preserved). A per-game list
replaces the shared one while it exists; switching back keeps it as
`<file>.off`, and switching to "this game only" copies the shared list first.

## Remembered choice

`launcher` / `launcher-<appid>`: the first line is the chosen `.exe`. An
existing per-game file wins even when empty; an empty file means "forgotten"
and must not fall through to the shared one.

## Run log

`mp-<game>-<appid>.log`, truncated on every run, written through **one**
`O_APPEND` handle that the children inherit (see `recipes.md`). Lines are
`[<rfc3339>] <key> : <value>` except the markers:

| line | meaning |
|---|---|
| `=== steam-mp-launch ===` | run header |
| `appid : …`, `game : …`, `proton : …`, `launcher (chosen) : …`, `compatdata : …` | context |
| `wait for game : no limit \| N s \| no (WAIT_FOR_GAME=0)` | effective setting |
| `sidecar wait : no wait \| N s` | effective `EXTRA_TIMEOUT` |
| `session up : pid N` | Proton is running |
| `game up : <exe>` | the launcher/game process appeared |
| `sidecar wait : timed out after Ns` | the program did not appear in time |
| `extra started : <path> (pid N)` / `extra missing : <path>` | sidecars |
| `extra wait : up \| did not appear` | game-direct start: sidecar breathing |
| `extras stopping : N` / `extras stopped : N` | shutdown (TERM, then KILL after 5 s) |
| `game gone : <base>` / `game wait : …` | the game ended or the wait timed out |
| `ERROR: <message>` | failed launch (also followed by the done marker) |
| `=== steam-mp-launch done (rc=N) ===` | last line, both success and failure |

The Logs page derives the status from `ERROR:`; the session tray reads the
session state and uses `done` as the fallback.

## JSON API

`mp_backend_call(handle, json)` takes one object with `"cmd"` and returns one
object with `ok` and either `error` or the payload. Commands: `picker`,
`manager`, `launch`, `run_extra`, `session`, `stop_session`, `icon`,
`list_add`, `list_remove`, `list_relink`, `list_rename`, `list_scope`,
`save_choice`, `forget`, `protons`, `set_proton`, `settings`, `set_setting`,
`restore_settings`, `logs`, `read_log`, `clear_logs`, `games`,
`install_find_installer`, `install_detect`, `install_prepare`,
`install_probe`, `install_close`, `install_finish`.

`launch` takes `exe`, `game_exe`, `compatdata`, `remember` and `sidecars`
(`true` by default; `false` starts no sidecars for that launch) and returns
`{session, id}` immediately; `session {id}` returns:

```
id, appid, game, exe, proton, log, started, game_up,
extras_started[], extras_skipped[], stopping, done, rc, error
```

`picker` also reports `installers: {truckersmp, trucksbook}` (whether the
launch button can be hidden), `cancel_mode`, `confirm_delay`, `launcher_gui`
and `fallback_to_game`.

`install_detect` reports `installed`, `installing`, `version`, `launcher`,
`client`, `game_exe`, `game_exe_win`, `has_s`; `install_probe` reports
`found`, `path`, `running`, `ready`, `complete`, `version`, `game_path`.

## Single-instance IPC

One JSON object per line over a per-user Unix socket. A second process sends
`{"mode":"pick"|"manage","args":{...}}` and waits for the reply:

| reply | meaning |
|---|---|
| `{"handled":true,"done":true}` | the manager request was adopted |
| `{"handled":true,"rc":0,"value":"<path>","sidecars":true}` | the picker chose a program (and whether to start sidecars) |
| `{"handled":true,"rc":1}` | the picker was cancelled |
| `{"handled":false}` | the window refused; start your own |

The picker helper prints the chosen program on the first line and
`--no-sidecars` on the second when its sidecar switch is off; it exits `1` on
cancel, and `mp-run` turns that into `0` (or into the plain game with
`GUI_CANCEL=game`).
