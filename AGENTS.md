# AGENTS.md — notes for coding agents

Steam MP Launcher is a Steam compatibility tool for Linux: the UI is C++20 with
Qt 6 Widgets, the launcher logic is Rust, and the two sides talk over a small C
ABI carrying JSON.

## Read first

- `ARCHITECTURE.md` — the boundary, the modules and the backend commands.
- `docs/contracts.md` — paths, file formats, JSON API, run log, IPC.
- `docs/recipes.md` — build/test, a session smoke test without Steam, the
  installers, the release commands and two pitfalls (log handle, process
  matching).

## Commands

```bash
./build.sh                                   # cargo + qmake6 (or cmake)
cargo test --manifest-path backend/Cargo.toml
cargo clippy --all-targets --manifest-path backend/Cargo.toml
./run.sh --manage                            # the UI
QT_QPA_PLATFORM=offscreen ./ui/steam-mp-launcher --shot /tmp/p.png
./install-tool.sh                            # install as a compatibility tool
./make-bundle.sh && ./make-dist.sh && ./make-dist.sh --source
```

## Rules

1. **One log handle.** `launch.rs` writes the run log through one `O_APPEND`
   handle and the children get a `dup` of it. A second handle overwrites the
   session lines — do not reintroduce one (symptoms: `docs/recipes.md`).
2. **Process matching** skips every ancestor and the wrappers (`reaper`,
   `steam-mp-launch`, `unshare`, `steam.exe`, `*/proton`, and
   `truckersmp-launch` when looking for the game). Without that the session
   waits for its own parent.
3. `EXTRA_TIMEOUT=0` means "do not wait"; `WAIT_FOR_GAME_TIMEOUT=0` means
   "no limit".
4. The log must end with `=== steam-mp-launch done (rc=N) ===` even on failure
   (`ERROR: …`); the tray reads the session state and the log is the fallback.
5. A behaviour change needs a test in `cargo test` (or `--shot` plus the
   session smoke test from `docs/recipes.md` for UI work) and a note in
   `docs/contracts.md` / README when it changes a contract.
6. **Bundle paths stay out of Proton.** The bundle's `LD_LIBRARY_PATH` and
   Qt plugin paths go to the UI only (`clean_qt_env`); `clean_proton_env`
   strips the bundle entries before Proton starts. A source install has no
   `usr/bin`/`usr/lib`, so both functions leave it alone.
7. Commits are short and prefixed by area (`launch:`, `ui:`, `install:`,
   `packaging:`, `docs:`).
