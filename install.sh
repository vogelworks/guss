#!/usr/bin/env sh
# Bootstrap `vhp` on a machine with no Rust toolchain.
#
#   curl -fsSL http://dev.vhpportal.office/tools/install.sh | sh
#
# This script is the trust-on-first-use step: it checks the sha256 from the
# manifest but cannot check the signature, because there is no trusted key on the
# machine yet. Every *subsequent* update goes through `vhp setup update`, which
# does verify signatures. Keep this file short — everything it cannot verify is
# risk, and everything else belongs in Rust where it is tested.
#
# Serve it from the same directory as the manifest, and set the two values below.
set -eu

BASE_URL="${VHP_BASE_URL:-http://dev.vhpportal.office/tools}"
CHANNEL="${VHP_CHANNEL:-prod}"
# Public half of the release signing key, from `vhp setup keygen` on the
# publishing machine. Written into the new install's config so that
# `vhp setup update` can verify signatures from here on.
RELEASE_PUBLIC_KEY="${VHP_RELEASE_KEY:-REPLACE_WITH_OUTPUT_OF_vhp_setup_keygen}"

PREFIX="${VHP_PREFIX:-$HOME/.local/bin}"
CHANNEL_URL="$BASE_URL/$CHANNEL"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "fetching $CHANNEL_URL/manifest.toml"
curl -fsSL "$CHANNEL_URL/manifest.toml" -o "$TMP/manifest.toml"

# The manifest is small and flat, so sed is enough and keeps this dependency-free.
value() { sed -n "s/^$1 *= *\"\\(.*\\)\"/\\1/p" "$TMP/manifest.toml"; }
VERSION="$(value version)"
FILE="$(value file)"
WANT_SHA="$(value sha256)"
[ -n "$FILE" ] && [ -n "$WANT_SHA" ] || { echo "manifest is missing file/sha256" >&2; exit 1; }

echo "downloading vhp $VERSION"
curl -fsSL "$CHANNEL_URL/$FILE" -o "$TMP/$FILE"

GOT_SHA="$(sha256sum "$TMP/$FILE" | cut -d' ' -f1)"
if [ "$GOT_SHA" != "$WANT_SHA" ]; then
    echo "sha256 mismatch: manifest $WANT_SHA, download $GOT_SHA" >&2
    exit 1
fi

tar xzf "$TMP/$FILE" -C "$TMP"
mkdir -p "$PREFIX"
install -m 0755 "$TMP/vhp" "$PREFIX/vhp"
echo "installed $PREFIX/vhp"

# Record where updates come from, plus the key that verifies them, before handing
# off to `setup init` for the rest (checkout path, shell function, submodules).
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/vhp"
mkdir -p "$CONFIG_DIR"
if ! grep -q '^\[release\]' "$CONFIG_DIR/config.toml" 2>/dev/null; then
    cat >>"$CONFIG_DIR/config.toml" <<EOF

[release]
base_url = "$BASE_URL"
ssh_host = ""
remote_dir = ""
public_key = "$RELEASE_PUBLIC_KEY"
EOF
    echo "wrote [release] to $CONFIG_DIR/config.toml"
fi

case ":$PATH:" in
    *":$PREFIX:"*) ;;
    *) echo "note: $PREFIX is not on your PATH" ;;
esac

echo
echo "Next: cd into your project-vhp checkout and run"
echo "  vhp setup init"
