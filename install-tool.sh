#!/usr/bin/env bash
# Install Steam MP Launcher as a Steam compatibility tool.
set -euo pipefail
HERE="$(cd -- "$(dirname -- "$0")" && pwd)"
TOOL_NAME="steam-mp-launcher-next"
cd "$HERE"

if [ "$(id -u)" -eq 0 ]; then
	echo "ERROR: run this as your normal user, not with sudo." >&2
	exit 1
fi

steam_dir() {
	if [ -n "${STEAM_COMPAT_CLIENT_INSTALL_PATH:-}" ] &&
		[ -d "$STEAM_COMPAT_CLIENT_INSTALL_PATH/steamapps" ]; then
		printf '%s' "${STEAM_COMPAT_CLIENT_INSTALL_PATH%/}"
		return 0
	fi
	for c in "$HOME/.local/share/Steam" "$HOME/.steam/root" "$HOME/.steam/steam" \
		"$HOME/.var/app/com.valvesoftware.Steam/data/Steam"; do
		if [ -d "$c/steamapps" ]; then
			(cd "$c" && pwd)
			return 0
		fi
	done
	return 1
}

STEAM="$(steam_dir)" || {
	echo "ERROR: could not find a Steam installation." >&2
	exit 1
}
case "$STEAM" in
*/.var/app/com.valvesoftware.Steam/*)
	# The Flatpak sandbox does not mount the host's Qt/KF6, so a source build
	# (which links against them) cannot start its UI there.
	cat >&2 <<'EOF'
ERROR: this is Flatpak Steam (~/.var/app/com.valvesoftware.Steam).

The Flatpak sandbox does not see the host Qt/KF6 libraries, so the UI built
here would not start inside it. Use the self-contained bundle instead:

  ./make-bundle.sh
  MP_BUNDLE=$PWD/dist/steam-mp-launcher-next-0.1.0-bundle-linux-x86_64.tar.gz ./install.sh

(or run the published installer: curl .../releases/latest/download/install.sh | bash)
EOF
	exit 1
	;;
esac
DEST="$STEAM/compatibilitytools.d/$TOOL_NAME"

echo "Steam        : $STEAM"
echo "Installing to: $DEST"

./build.sh

mkdir -p "$DEST"
install -m 0755 backend/target/release/mp-run "$DEST/steam-mp-launch"
install -m 0755 ui/steam-mp-launcher "$DEST/steam-mp-launcher"
install -m 0644 dist/steam-mp-launcher.svg "$DEST/steam-mp-launcher.svg"
install -m 0644 tool/compatibilitytool.vdf "$DEST/compatibilitytool.vdf"
install -m 0644 tool/toolmanifest.vdf "$DEST/toolmanifest.vdf"

DATA="$HOME/.local/share"
APPS="$DATA/applications"
ICONS="$DATA/icons/hicolor/scalable/apps"
META="$DATA/metainfo"
BIN="$HOME/.local/bin"
mkdir -p "$APPS" "$ICONS" "$META" "$BIN"

install -m 0644 dist/steam-mp-launcher.svg "$ICONS/steam-mp-launcher.svg"
install -m 0644 dist/steam-mp-launcher-symbolic.svg \
	"$ICONS/steam-mp-launcher-symbolic.svg"
install -m 0644 dist/io.github.steam_mp_launcher.SteamMpLauncher.metainfo.xml \
	"$META/io.github.steam_mp_launcher.SteamMpLauncher.metainfo.xml"
sed -e "s|^Exec=.*|Exec=$DEST/steam-mp-launcher|" \
	-e "s|^TryExec=.*|TryExec=$DEST/steam-mp-launcher|" \
	dist/io.github.steam_mp_launcher.SteamMpLauncher.desktop \
	> "$APPS/io.github.steam_mp_launcher.SteamMpLauncher.desktop"
ln -sfn "$DEST/steam-mp-launcher" "$BIN/steam-mp-launcher"

cat > "$APPS/steam-mp-launcher-next-open.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Steam MP Launcher (Next) - Open with
Exec=$DEST/steam-mp-launcher --open %f
TryExec=$DEST/steam-mp-launcher
Icon=steam-mp-launcher
MimeType=application/x-ms-dos-executable;application/x-msdownload;application/vnd.microsoft.portable-executable;
Terminal=false
NoDisplay=true
DESKTOP

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" >/dev/null 2>&1 || true
command -v kbuildsycoca6 >/dev/null 2>&1 && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$DATA/icons/hicolor" >/dev/null 2>&1 || true

echo
echo "Installed 'Steam MP Launcher (Next)'."
echo "Restart Steam, then:"
echo "  ETS2 -> Properties -> Compatibility"
echo "  -> Force the use of a specific Steam Play compatibility tool"
echo "  -> Steam MP Launcher (Next)"
echo
echo "The first launch opens the picker."
