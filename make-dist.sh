#!/usr/bin/env bash
# Build a tarball a friend can unpack and install without a toolchain.
#
#   ./make-dist.sh            # -> dist/steam-mp-launcher-next-<ver>-linux-x86_64.tar.gz
#   ./make-dist.sh --source   # -> dist/steam-mp-launcher-next-<ver>-src.tar.gz
set -euo pipefail
HERE="$(cd -- "$(dirname -- "$0")" && pwd)"
cd "$HERE"

VERSION="0.1.0"
NAME="steam-mp-launcher-next-$VERSION"
DIST="$HERE/dist"
mkdir -p "$DIST"

if [ "${1:-}" = "--source" ]; then
	# Stage outside the tree: writing into dist while tarring it would make
	# tar read the file it is creating.
	SRC="$(mktemp -d)/$NAME-src"
	mkdir -p "$SRC"
	tar -C "$HERE" -cf - \
		--exclude='./.git' \
		--exclude='./backend/target' \
		--exclude='./build' \
		--exclude='./ui/*.o' \
		--exclude='./ui/moc_*' \
		--exclude='./ui/Makefile' \
		--exclude='./ui/.qmake.stash' \
		--exclude='./ui/steam-mp-launcher' \
		--exclude='./dist/*.tar.gz' \
		--exclude='./dist/appdir' \
		--exclude='./dist/SHA256SUMS' \
		. | tar -C "$SRC" -xf -
	tar -C "$(dirname "$SRC")" -czf "$DIST/$NAME-src.tar.gz" "$NAME-src"
	rm -rf "$(dirname "$SRC")"
	echo "$DIST/$NAME-src.tar.gz"
	exit 0
fi

./build.sh >/dev/null

STAGE="$DIST/$NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE/tool" "$STAGE/dist"

install -m 0755 backend/target/release/mp-run "$STAGE/tool/steam-mp-launch"
install -m 0755 ui/steam-mp-launcher "$STAGE/tool/steam-mp-launcher"
install -m 0644 tool/compatibilitytool.vdf tool/toolmanifest.vdf "$STAGE/tool/"
install -m 0644 dist/steam-mp-launcher.svg dist/steam-mp-launcher-symbolic.svg \
	dist/io.github.steam_mp_launcher.SteamMpLauncher.desktop \
	dist/steam-mp-launcher-next-open.desktop \
	dist/io.github.steam_mp_launcher.SteamMpLauncher.metainfo.xml "$STAGE/dist/"

cat > "$STAGE/install.sh" <<'INSTALL'
#!/usr/bin/env bash
# Steam MP Launcher (Next) - install the prebuilt files as a second
# compatibility tool.  Run as your normal user, with Steam closed.
set -euo pipefail
HERE="$(cd -- "$(dirname -- "$0")" && pwd)"
TOOL_NAME="steam-mp-launcher-next"
DATA="$HOME/.local/share"
DEST=""

if [ "$(id -u)" -eq 0 ]; then
	echo "ERROR: run this as your normal user, not with sudo." >&2
	exit 1
fi

for c in "${STEAM_COMPAT_CLIENT_INSTALL_PATH:-}" "$HOME/.local/share/Steam" \
	"$HOME/.steam/root" "$HOME/.steam/steam" \
	"$HOME/.var/app/com.valvesoftware.Steam/data/Steam"; do
	[ -n "$c" ] && [ -d "$c/steamapps" ] && { DEST="$c/compatibilitytools.d/$TOOL_NAME"; break; }
done
[ -n "$DEST" ] || { echo "ERROR: could not find a Steam installation." >&2; exit 1; }

mkdir -p "$DEST"
install -m 0755 "$HERE/tool/steam-mp-launch" "$DEST/steam-mp-launch"
install -m 0755 "$HERE/tool/steam-mp-launcher" "$DEST/steam-mp-launcher"
install -m 0644 "$HERE/tool/compatibilitytool.vdf" "$DEST/compatibilitytool.vdf"
install -m 0644 "$HERE/tool/toolmanifest.vdf" "$DEST/toolmanifest.vdf"
install -m 0644 "$HERE/dist/steam-mp-launcher.svg" "$DEST/steam-mp-launcher.svg"

APPS="$DATA/applications"; ICONS="$DATA/icons/hicolor/scalable/apps"
META="$DATA/metainfo"; BIN="$HOME/.local/bin"
mkdir -p "$APPS" "$ICONS" "$META" "$BIN"
install -m 0644 "$HERE/dist/steam-mp-launcher.svg" "$ICONS/steam-mp-launcher.svg"
install -m 0644 "$HERE/dist/steam-mp-launcher-symbolic.svg" "$ICONS/steam-mp-launcher-symbolic.svg"
install -m 0644 "$HERE/dist/io.github.steam_mp_launcher.SteamMpLauncher.metainfo.xml" \
	"$META/io.github.steam_mp_launcher.SteamMpLauncher.metainfo.xml"
sed -e "s|^Exec=.*|Exec=$DEST/steam-mp-launcher|" \
	-e "s|^TryExec=.*|TryExec=$DEST/steam-mp-launcher|" \
	"$HERE/dist/io.github.steam_mp_launcher.SteamMpLauncher.desktop" \
	> "$APPS/io.github.steam_mp_launcher.SteamMpLauncher.desktop"
sed -e "s|^Exec=.*|Exec=$DEST/steam-mp-launcher --open %f|" \
	-e "s|^TryExec=.*|TryExec=$DEST/steam-mp-launcher|" \
	"$HERE/dist/steam-mp-launcher-next-open.desktop" \
	> "$APPS/steam-mp-launcher-next-open.desktop"
ln -sfn "$DEST/steam-mp-launcher" "$BIN/steam-mp-launcher"

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" >/dev/null 2>&1 || true
command -v kbuildsycoca6 >/dev/null 2>&1 && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$DATA/icons/hicolor" >/dev/null 2>&1 || true

echo "Installed to: $DEST"
echo "Restart Steam, then: ETS2 -> Properties -> Compatibility ->"
echo "  Force the use of a specific Steam Play compatibility tool -> Steam MP Launcher (Next)"
INSTALL
chmod 0755 "$STAGE/install.sh"

cat > "$STAGE/README.txt" <<'README'
Steam MP Launcher (Next) - prebuilt Linux x86_64
================================================

What it is: a Steam compatibility tool that starts ETS2/ATS (or any game)
through Proton together with a multiplayer launcher (TruckersMP, TrucksBook).
The UI is Qt 6 Widgets; everything else is a Rust library inside the binary.

1. Dependencies (runtime only, no Rust needed)
---------------------------------------------
Qt 6: Core, Gui, Widgets, Network, DBus + the SVG icon plugin.
KF6StatusNotifierItem (+ KF6WindowSystem) for the tray.

Arch/CachyOS:
  sudo pacman -S qt6-base qt6-wayland qt6-svg kstatusnotifieritem

Debian/Ubuntu (KF6 packages exist in Debian 13 / Ubuntu 24.10+):
  sudo apt install qt6-base-dev qt6-wayland qt6-svg-dev \
      libkf6statusnotifieritem6 libkf6windowsystem6

Fedora:
  sudo dnf install qt6-qtbase qt6-qtwayland qt6-qtsvg \
      kf6-kstatusnotifieritem kf6-kwindowsystem

openSUSE:
  sudo zypper install qt6-base qt6-wayland qt6-svg \
      kf6-statusnotifieritem kf6-kwindowsystem

The binary was built on a rolling release; if it complains about glibc,
build from source instead (see the -src tarball).

2. Install
----------
  tar -xzf steam-mp-launcher-next-0.1.0-linux-x86_64.tar.gz
  cd steam-mp-launcher-next-0.1.0
  ./install.sh

Then restart Steam:
  ETS2 -> Properties -> Compatibility
  -> Force the use of a specific Steam Play compatibility tool
  -> Steam MP Launcher (Next)


3. Notes
--------
- Config: ~/.config/steam-mp-launcher/
- Logs:   ~/.local/state/steam-mp-launcher/logs/
- First launch of the picker asks which launcher to run; "Manage..." has the
  lists, Proton selection, settings and logs.
- TruckersMP install: picker -> "Install TruckersMP" walks through the setup
  (Download -> Run -> Finish -> Added) and writes the game path itself.
README

tar -C "$DIST" -czf "$DIST/$NAME-linux-x86_64.tar.gz" "$NAME"
rm -rf "$STAGE"
echo "$DIST/$NAME-linux-x86_64.tar.gz"
