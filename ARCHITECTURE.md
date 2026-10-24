# Steam MP Launcher — architecture

**All UI in C++20/Qt 6 Widgets, all launcher logic in Rust**, joined by a small
C ABI carrying JSON.

```
ui/        C++20 + Qt 6 Widgets + KF6StatusNotifierItem (tray only)
backend/   Rust library (config, lists, Proton, launch, logs) + debug CLI
  staticlib -> ui links libmp_backend.a through a small C ABI
  JSON request/response keeps the boundary stable and testable
build.sh   CMake (preferred; qmake6 + cargo fallback)
run.sh     launch the UI
```

## Boundary

C++ calls one function:

```c
char *mp_backend_call(MpBackend *b, const char *json_request);
void  mp_backend_free_string(char *reply);
```

Every request is `{"cmd":"...", ...}`; every response is a JSON object with
`ok: true/false` and an optional `error`. The reply string is owned by the
caller (`mp_backend_free_string` releases it). The C++ wrapper parses it into
`QJsonObject`; no Qt types cross the ABI. `launch` returns a `session` id
immediately and runs in a Rust thread; the tray polls
`{"cmd":"session","id":N}`.

## Backend modules

| File | Responsibility |
|---|---|
| `paths.rs` | XDG dirs, MP_* env, per-game file names |
| `config.rs` | conf parser + `conf_set` with comments preserved |
| `lists.rs` | `label|path`, atomic writes, add/remove/relink, scopes |
| `saved.rs` | `launcher` / `launcher-<appid>` |
| `protons.rs` | Steam libraries + distro `compatibilitytools.d`, scoring |
| `games.rs` | appmanifest/config.vdf detection, exe lookup |
| `steam_art.rs` | library hero + square icon from `appcache` |
| `icons.rs` | `.exe` icon from PE resources, cached as `.ico`/`.png` |
| `settings.rs` | the settings shown in the UI (normalize/get/set) |
| `logs.rs` | run list, read, clear, session log parsing |
| `launch.rs` | Proton process group, sidecars, session state |
| `install.rs` | installer cleanup: find/close launcher, Squirrel install state, fix game path, list entry |
| `api.rs` | command dispatch (JSON) |
| `lib.rs` / `main.rs` | C ABI / debug CLI |
| `bin/mp_run.rs` | Steam compatibility-tool verbs + picker helper |

## Steam entry point

`mp-run` (installed as `steam-mp-launch`) implements `run`,
`waitforexitandrun`, `runinprefix`, `getcompatpath` and `getnativepath`.
It invokes the C++ UI in `--pick` helper mode, launches the chosen launcher or
the game with plain `proton run` (Steam/Proton own the prefix, drives and
DXVK), and blocks on the session. `install-tool.sh` installs it as the
`steam-mp-launcher-next` compatibility tool.

## Non-goals

Full translation catalogs and an accessibility audit. The installer flow is
compact (no download auto-close/quarantine steps).

The commands, the run log, the process-matching rules and the file layout are
in [`docs/contracts.md`](docs/contracts.md) and
[`docs/recipes.md`](docs/recipes.md).
