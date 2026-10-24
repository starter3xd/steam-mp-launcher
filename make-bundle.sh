#!/usr/bin/env bash
# Build the self-contained bundle: Steam needs a directory with the VDFs, and
# inside a Flatpak Steam sandbox (or on a machine without Qt) the host Qt is not
# visible, so every library travels with the tool. install.sh downloads this
# tarball and unpacks it into compatibilitytools.d.
set -euo pipefail
HERE="$(cd -- "$(dirname -- "$0")" && pwd)"
cd "$HERE"

VERSION="0.1.0"
NAME="steam-mp-launcher-next-$VERSION"
DIST="$HERE/dist"
BUILD="$DIST/appdir"
mkdir -p "$DIST"

./build.sh >/dev/null

rm -rf "$BUILD"
mkdir -p "$BUILD/usr/bin" "$BUILD/usr/lib" "$BUILD/usr/plugins" \
	"$BUILD/usr/share/icons/hicolor/scalable/apps"

install -m 0755 backend/target/release/mp-run "$BUILD/usr/bin/steam-mp-launch"
install -m 0755 ui/steam-mp-launcher "$BUILD/usr/bin/steam-mp-launcher"
mkdir -p "$BUILD/usr/share/icons/hicolor/scalable/apps" "$BUILD/usr/share/applications" \
	"$BUILD/usr/share/metainfo"
install -m 0644 dist/steam-mp-launcher.svg dist/steam-mp-launcher-symbolic.svg \
	"$BUILD/usr/share/icons/hicolor/scalable/apps/"
# The same desktop entry, metainfo and icons the source installer puts into
# ~/.local/share; install.sh copies them after the first start.
install -m 0644 dist/io.github.steam_mp_launcher.SteamMpLauncher.desktop \
	"$BUILD/usr/share/applications/"
install -m 0644 dist/io.github.steam_mp_launcher.SteamMpLauncher.metainfo.xml \
	"$BUILD/usr/share/metainfo/"
# Qt finds its plugins next to the binary (usr/bin/../plugins); the Qt libs come
# from the $ORIGIN/../lib RUNPATH, so no LD_LIBRARY_PATH is needed anywhere.
printf '[Paths]\nPrefix=..\n' > "$BUILD/usr/bin/qt.conf"

# Bundle every non-base library both binaries need (Qt 6, KF6, ICU, ...).
copy_libs() {
	local binary="$1"
	ldd "$binary" 2>/dev/null | awk '/=> \// {print $3}' | while read -r lib; do
		case "$(basename "$lib")" in
		libc.so.* | libm.so.* | libdl.so.* | libpthread.so.* | librt.so.* | \
			libresolv.so.* | ld-linux* | linux-vdso*) continue ;;
		esac
		[ -e "$BUILD/usr/lib/$(basename "$lib")" ] || cp -L "$lib" "$BUILD/usr/lib/"
	done
}
copy_libs ui/steam-mp-launcher
copy_libs backend/target/release/mp-run

# Qt plugins: wayland/xcb platform, SVG icon engine, image formats and the
# Wayland client integrations.  Without these a bundled Qt starts but cannot
# create a window or render the SVG icons.
copy_plugins() {
	local src="$1" dst="$2"
	[ -d "$src" ] || return 0
	mkdir -p "$BUILD/usr/plugins/$dst"
	cp -L "$src"/*.so "$BUILD/usr/plugins/$dst/" 2>/dev/null || true
}
QT_PLUGINS=/usr/lib/qt6/plugins
copy_plugins "$QT_PLUGINS/platforms" platforms
copy_plugins "$QT_PLUGINS/iconengines" iconengines
copy_plugins "$QT_PLUGINS/imageformats" imageformats
copy_plugins "$QT_PLUGINS/wayland-shell-integration" wayland-shell-integration
copy_plugins "$QT_PLUGINS/wayland-decoration-client" wayland-decoration-client
copy_plugins "$QT_PLUGINS/wayland-graphics-integration-client" wayland-graphics-integration-client
copy_plugins "$QT_PLUGINS/xcbglintegrations" xcbglintegrations
for plugin in "$BUILD"/usr/plugins/*/*.so; do
	[ -e "$plugin" ] || continue
	ldd "$plugin" 2>/dev/null | awk '/=> \// {print $3}' | while read -r lib; do
		case "$(basename "$lib")" in
		libc.so.* | libm.so.* | libdl.so.* | libpthread.so.* | librt.so.* | \
			libresolv.so.* | ld-linux* | linux-vdso*) continue ;;
		esac
		[ -e "$BUILD/usr/lib/$(basename "$lib")" ] || cp -L "$lib" "$BUILD/usr/lib/"
	done
done

cat > "$BUILD/AppRun" <<'APPRUN'
#!/bin/sh
# One entry point for both roles: Steam passes compatibility-tool verbs, the
# desktop launcher passes normal Qt arguments.
HERE="$(dirname "$(readlink -f "$0")")"
# No LD_LIBRARY_PATH/QT_PLUGIN_PATH here: those would leak into Proton and the
# game. The UI binary carries a $ORIGIN/../lib RUNPATH and usr/bin/qt.conf for
# its plugins instead.
export XDG_DATA_DIRS="$HERE/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"

case "${1:-}" in
run | waitforexitandrun | runinprefix | getcompatpath | getnativepath)
	exec "$HERE/usr/bin/steam-mp-launch" "$@" ;;
*)
	exec "$HERE/usr/bin/steam-mp-launcher" "$@" ;;
esac
APPRUN
chmod 0755 "$BUILD/AppRun"

install -m 0644 tool/compatibilitytool.vdf tool/toolmanifest.vdf "$BUILD/"
cat > "$BUILD/steam-mp-launch" <<'WRAP'
#!/bin/sh
exec "$(dirname "$(readlink -f "$0")")/AppRun" "$@"
WRAP
cat > "$BUILD/steam-mp-launcher" <<'WRAP2'
#!/bin/sh
exec "$(dirname "$(readlink -f "$0")")/AppRun" "$@"
WRAP2
chmod 0755 "$BUILD/steam-mp-launch" "$BUILD/steam-mp-launcher"

cat > "$BUILD/install.sh" <<'INSTALL'
#!/usr/bin/env bash
# Install Steam MP Launcher (Next) as a compatibility tool from this bundle.
# A version stamp makes a repeated run a no-op (use --force to refresh).
set -euo pipefail
SRC="${1:-$(cd -- "$(dirname -- "$0")" && pwd)}"
shift || true
QUIET=0
FORCE=0
for arg in "$@"; do
	case "$arg" in
	--quiet) QUIET=1 ;;
	--force) FORCE=1 ;;
	esac
done
VERSION="0.1.0"
TOOL_NAME="steam-mp-launcher-next"
DEST=""
for c in "${STEAM_COMPAT_CLIENT_INSTALL_PATH:-}" "$HOME/.local/share/Steam" \
	"$HOME/.steam/root" "$HOME/.steam/steam" \
	"$HOME/.var/app/com.valvesoftware.Steam/data/Steam"; do
	[ -n "$c" ] && [ -d "$c/steamapps" ] && { DEST="$c/compatibilitytools.d/$TOOL_NAME"; break; }
done
[ -n "$DEST" ] || { [ "$QUIET" = 1 ] || echo "ERROR: could not find a Steam installation." >&2; exit 1; }

if [ "$FORCE" = 0 ] && [ -f "$DEST/.install-version" ] &&
	[ "$(cat "$DEST/.install-version")" = "$VERSION" ]; then
	[ "$QUIET" = 1 ] || echo "Already installed: $DEST"
	exit 0
fi

mkdir -p "$DEST"
cp -a "$SRC"/. "$DEST"/
printf '%s' "$VERSION" > "$DEST/.install-version"

# Desktop integration, the same files install-tool.sh installs.
DATA="$HOME/.local/share"
APPS="$DATA/applications"; ICONS="$DATA/icons/hicolor/scalable/apps"
META="$DATA/metainfo"; BIN="$HOME/.local/bin"
mkdir -p "$APPS" "$ICONS" "$META" "$BIN"
if [ -f "$DEST/usr/share/applications/io.github.steam_mp_launcher.SteamMpLauncher.desktop" ]; then
	sed -e "s|^Exec=.*|Exec=$DEST/steam-mp-launcher|" \
		-e "s|^TryExec=.*|TryExec=$DEST/steam-mp-launcher|" \
		"$DEST/usr/share/applications/io.github.steam_mp_launcher.SteamMpLauncher.desktop" \
		> "$APPS/io.github.steam_mp_launcher.SteamMpLauncher.desktop"
fi
cat > "$APPS/steam-mp-launcher-next-open.desktop" <<OPENWITH
[Desktop Entry]
Type=Application
Name=Steam MP Launcher (Next) - Open with
Exec=$DEST/steam-mp-launcher --open %f
TryExec=$DEST/steam-mp-launcher
Icon=steam-mp-launcher
MimeType=application/x-ms-dos-executable;application/x-msdownload;application/vnd.microsoft.portable-executable;
Terminal=false
NoDisplay=true
OPENWITH
[ -d "$DEST/usr/share/icons/hicolor/scalable/apps" ] &&
	install -m 0644 "$DEST"/usr/share/icons/hicolor/scalable/apps/*.svg "$ICONS/"
[ -d "$DEST/usr/share/metainfo" ] &&
	install -m 0644 "$DEST"/usr/share/metainfo/*.xml "$META/"
ln -sfn "$DEST/steam-mp-launcher" "$BIN/steam-mp-launcher"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" >/dev/null 2>&1 || true
command -v kbuildsycoca6 >/dev/null 2>&1 && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$DATA/icons/hicolor" >/dev/null 2>&1 || true

echo "Installed to: $DEST"
echo "Restart Steam, then: ETS2 -> Properties -> Compatibility"
echo "  -> Force the use of a specific Steam Play compatibility tool -> Steam MP Launcher (Next)"
if [ "$QUIET" = 1 ] && command -v notify-send >/dev/null 2>&1; then
	# A host tool: keep the bundle's libraries out of it.
	env -u LD_LIBRARY_PATH -u QT_PLUGIN_PATH -u QT_QPA_PLATFORM_PLUGIN_PATH \
		notify-send "Steam MP Launcher" "Installed as a Steam compatibility tool. Restart Steam to select it." || true
fi
INSTALL
chmod 0755 "$BUILD/install.sh"

# --- bundle for Steam -----------------------------------------------------
BUNDLE="$DIST/$NAME-bundle"
rm -rf "$BUNDLE"
cp -a "$BUILD" "$BUNDLE"

tar -C "$DIST" -czf "$DIST/$NAME-bundle-linux-x86_64.tar.gz" "$NAME-bundle"
rm -rf "$BUNDLE"
echo "bundle: $DIST/$NAME-bundle-linux-x86_64.tar.gz"
