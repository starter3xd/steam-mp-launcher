#!/usr/bin/env bash
# Steam MP Launcher installer.
#
#   curl -fsSL https://github.com/starter3xd/steam-mp-launcher/releases/latest/download/install.sh | bash
#
# Downloads the self-contained bundle (Qt/KF6 inside), installs it into Steam's
# compatibilitytools.d and adds the menu/"Open with" entries. No system
# packages and no sudo are needed.
#
# Offline or testing:
#   MP_BUNDLE=dist/steam-mp-launcher-next-0.1.0-bundle-linux-x86_64.tar.gz ./install.sh
set -euo pipefail

VERSION="0.1.0"
NAME="steam-mp-launcher-next-$VERSION-bundle-linux-x86_64.tar.gz"
RELEASE="${MP_RELEASE:-https://github.com/starter3xd/steam-mp-launcher/releases/download/v$VERSION}"
BUNDLE="${MP_BUNDLE:-$RELEASE/$NAME}"
SUMS="${MP_SUMS:-}"
if [ -z "$SUMS" ] && [ -z "${MP_BUNDLE:-}" ]; then
	SUMS="$RELEASE/SHA256SUMS"
fi

say() { printf '%s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] && die "run this as your normal user, not with sudo."

# Steam must exist; the bundle's install.sh picks the same root again.
STEAM=""
for c in "${STEAM_COMPAT_CLIENT_INSTALL_PATH:-}" "$HOME/.local/share/Steam" \
	"$HOME/.steam/root" "$HOME/.steam/steam" \
	"$HOME/.var/app/com.valvesoftware.Steam/data/Steam"; do
	if [ -n "$c" ] && [ -d "$c/steamapps" ]; then
		STEAM="$c"
		break
	fi
done
[ -n "$STEAM" ] || die "could not find Steam (native or Flatpak). Is Steam installed?"

# The bundle does not ship glibc: every current distro has 2.39, older ones do not.
GLIBC="$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')"
if [ -n "$GLIBC" ] && [ "$(printf '%s\n2.39\n' "$GLIBC" | sort -V | head -n1)" != "2.39" ]; then
	die "glibc $GLIBC is too old: the bundle needs 2.39 or newer (update your distribution)."
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if [ -f "$BUNDLE" ]; then
	cp "$BUNDLE" "$TMP/$NAME"
else
	if command -v curl >/dev/null 2>&1; then
		say "Downloading $NAME ..."
		curl -fL --progress-bar -o "$TMP/$NAME" "$BUNDLE" || die "download failed: $BUNDLE"
	elif command -v wget >/dev/null 2>&1; then
		say "Downloading $NAME ..."
		wget -q --show-progress -O "$TMP/$NAME" "$BUNDLE" || die "download failed: $BUNDLE"
	else
		die "curl or wget is required to download the bundle."
	fi
fi

# Verify against the release checksums when they are available.
if [ -n "$SUMS" ]; then
	if [ -f "$SUMS" ]; then
		cp "$SUMS" "$TMP/SHA256SUMS"
	elif command -v curl >/dev/null 2>&1; then
		curl -fsSL -o "$TMP/SHA256SUMS" "$SUMS" 2>/dev/null || rm -f "$TMP/SHA256SUMS"
	fi
fi
if [ -s "$TMP/SHA256SUMS" ] && command -v sha256sum >/dev/null 2>&1; then
	( cd "$TMP" && grep "  $NAME\$" SHA256SUMS | sha256sum -c - ) || die "checksum mismatch for $NAME."
fi

tar -xzf "$TMP/$NAME" -C "$TMP"
SRC="$TMP/steam-mp-launcher-next-$VERSION-bundle"
[ -x "$SRC/install.sh" ] || die "the bundle has no install.sh."
"$SRC/install.sh" "$SRC"

say
say "Done. Restart Steam, then: <game> -> Properties -> Compatibility"
say "  -> Force the use of a specific Steam Play compatibility tool"
say "  -> Steam MP Launcher (Next)"
