# Recipes

Reproducible operations for Steam MP Launcher. Run everything from the repository
root.

## Build and test

```bash
./build.sh                                   # cargo + qmake6 (or cmake when present)
cargo test --manifest-path backend/Cargo.toml
cargo clippy --all-targets --manifest-path backend/Cargo.toml
./run.sh --manage --page logs                # run the manager
```

The UI has no unit tests; after a window change take a screenshot (headless):

```bash
QT_QPA_PLATFORM=offscreen ./ui/steam-mp-launcher --shot /tmp/picker.png
QT_QPA_PLATFORM=offscreen ./ui/steam-mp-launcher --manage --page settings --shot /tmp/settings.png
```

`ui/demo-out/picker.png` is the picker with the Breeze window frame. `demo/`
points at this machine's ETS2 prefix (TruckersMP + TrucksBook), so the real
`.exe` icons appear:

```bash
MP_CONF=$PWD/demo/conf MP_LIST=$PWD/demo/launchers.list MP_EXTRAS=$PWD/demo/extras.list \
MP_SAVED=$PWD/demo/saved SteamAppId=227300 ./ui/steam-mp-launcher --pick --appid 227300 &
sleep 2
spectacle -b -n -a -o ui/demo-out/picker.png   # -a = active window
kill %1
```

`--shot` gives the same window without the frame
(`QT_QPA_PLATFORM=offscreen ./ui/steam-mp-launcher --shot /tmp/p.png`); a stale
icon means `rm -rf ~/.cache/steam-mp-launcher/icons` and another shot.

## Smoke-test a session without Steam

A fake `proton` plus a fake program exercise the whole launch path (session
log, sidecars, waits) without touching a real prefix:

```bash
T=/tmp/mp-session; rm -rf "$T"; mkdir -p "$T/conf" "$T/state" "$T/compatdata/227300"
printf '#!/bin/sh\nshift\nexec "$@"\n' > "$T/proton"; chmod +x "$T/proton"
printf '#!/bin/sh\necho "child output"; sleep 1\n' > "$T/app.exe"; chmod +x "$T/app.exe"
printf 'PROTON=%s\nWAIT_FOR_GAME=0\nEXTRA_DELAY=0\n' "$T/proton" > "$T/conf/steam-mp-launcher.conf"

MP_CONF=$T/conf/steam-mp-launcher.conf XDG_STATE_HOME=$T/state XDG_CONFIG_HOME=$T/xdg \
  ./backend/target/release/mp-backend raw \
  "{\"cmd\":\"launch\",\"appid\":\"227300\",\"exe\":\"$T/app.exe\",\"compatdata\":\"$T/compatdata/227300\"}"

grep -E 'session up|ERROR|done \(rc=|child output' "$T/state/steam-mp-launcher/logs/"*.log
```

The log must contain the child output **and** `session up` **and** the done
marker. A broken `PROTON` (e.g. a script with a missing interpreter) must give
`ERROR: …` plus `done (rc=1)`, and `mp-backend logs` must report
`status: "error"`.

## Pitfalls that cost hours

**One log handle.** `launch.rs` opens the log with `O_APPEND` and hands a
`dup` of that handle to the children. A second, independently opened handle
has its own offset: the child writes over `session up`, `game up`,
`extra started` and sometimes `done`. Symptom: the log starts with the header,
then only Proton output, and the tray/picker never come back.

**Never trust a bare `argv` match.** Steam keeps `<game.exe>` in the command
line of its own wrappers (`steam-runtime-launch-client`,
`steam-runtime-supervisor`, `reaper`, `steam-launch-wrapper`) and those are
*ancestors* of the tool. Matching them makes the session wait for its own
parent (it never ends) and fires `game up` before the game starts. The code
skips every ancestor (`/proc/<pid>/stat` → `PPid`) and the `reaper`,
`steam-mp-launch`, `unshare`, `steam.exe` and `*/proton` wrappers, plus
`truckersmp-launch` when looking for the game.

**Bundle paths stay out of Proton.** The bundle exports its
`LD_LIBRARY_PATH` and `QT_PLUGIN_PATH` for the UI; `mp-run` passes those to the
UI (`clean_qt_env`, because Steam's loader env is stripped) and
`clean_proton_env` removes the bundle entries plus the Qt variables before
Proton starts. Exporting them globally made Proton/Wine load the bundled
Qt/ICU libs.

Also remember: `EXTRA_TIMEOUT=0` means "do not wait", while
`WAIT_FOR_GAME_TIMEOUT=0` means "no limit".

## Installers

TruckersMP (`install_detect`/`install_probe`): the wizard runs the setup in
the game's prefix and does **not** trust the first `TruckersMP-Launcher.exe` —
Squirrel writes a ~396 kB stub ~17 s before the ~211 MB app is complete.

```bash
./backend/target/release/mp-backend raw '{"cmd":"install_detect","kind":"truckersmp","appid":"227300"}'
./backend/target/release/mp-backend raw '{"cmd":"install_probe","kind":"truckersmp","appid":"227300"}'
```

Complete means: `app-<max>/TruckersMP-Launcher.exe` ≥ 200 MB **and** the stub
**and** `packages/RELEASES` **and** (`app.ico` or the last `Squirrel-Install.log`
line `Finished Squirrel Updater`) **and** no `Update.exe`/`Setup.exe` of that
prefix. `--silent` would disable `--squirrel-firstrun`, so the wizard runs the
setup without arguments.

TrucksBook: the setup has a custom "select the game" page and cannot install
silently. The dialog finds the PE-version-string match in `Downloads`, shows
the Windows path of the game `.exe` (S:, or Z: when the prefix has no S:), and
adds `TB Client.exe` to the sidecars. `install_prepare` validates ZIP entries
(`..`, absolute paths, drive letters) before unpacking.

## Release

```bash
./make-bundle.sh               # self-contained bundle (what install.sh downloads)
./make-dist.sh                 # prebuilt tool + install.sh (needs system Qt)
./make-dist.sh --source        # sources (build.sh / install-tool.sh)
cd dist && sha256sum *-0.1.0-*.tar.gz >SHA256SUMS && sha256sum -c SHA256SUMS
```

Release assets: `install.sh`, the bundle, the prebuilt tarball, the source
tarball and `SHA256SUMS`. The installer downloads the bundle, verifies it and
runs the bundle's `install.sh` (menu entry, "Open with", icons, metainfo).

## Flatpak Steam

The installers look at `~/.var/app/com.valvesoftware.Steam/data/Steam` as well,
but only the **self-contained bundle** works there: the Flatpak sandbox mounts
neither the host Qt/KF6 nor the systray library, so a source build's UI cannot
start. `install-tool.sh` refuses a Flatpak Steam on purpose — use the bundle:

```bash
./make-bundle.sh
MP_BUNDLE=$PWD/dist/steam-mp-launcher-next-0.1.0-bundle-linux-x86_64.tar.gz ./install.sh
```

- The bundle needs **glibc ≥ 2.39** (measured: the UI and runner import up to
  `GLIBC_2.39`); current Steam Flatpak runtimes are 24.08 (2.39) and 25.08
  (2.42), so they are fine. An old 23.08 runtime (2.37) is too old.
- Flatpak uses a PID namespace; `/proc` shows the sandbox processes (Steam, the
  launcher, the game), which is exactly what the session tracker needs.
- The bundle installs the same desktop integration as `install-tool.sh`
  (menu entry, "Open with", icons, metainfo, `~/.local/bin` symlink).

## Debugging

```bash
ls -t ~/.local/state/steam-mp-launcher/logs | head
tail -n 40 ~/.local/state/steam-mp-launcher/logs/mp-<game>-<appid>.log
ps -eo pid,args | grep -E 'proton|reaper|steam-mp-launch|eurotrucks2'
```

