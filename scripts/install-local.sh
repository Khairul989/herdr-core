#!/bin/sh
# Builds a release binary into ~/.local/bin (replacing any older one), then
# restarts the core the planned way so running terminals pick up the new build
# (never handing this shell's environment to a core it has to start).
set -eu
cd "$(dirname "$0")/.."
os=$(uname -s)
if [ "$os" = Darwin ]; then
  config="${XDG_CONFIG_HOME:-$HOME/.config}/herdr-core/local-signing.env"
  if [ -z "${HC_MACOS_BUNDLE_ID+x}${HC_MACOS_TEAM_ID+x}${HC_MACOS_SIGN_IDENTITY+x}" ] && [ -f "$config" ]; then
    set -a
    . "$config"
    set +a
  fi
  : "${HC_MACOS_BUNDLE_ID:?configure a distinct HC_MACOS_BUNDLE_ID in $config}"
  : "${HC_MACOS_TEAM_ID:?configure HC_MACOS_TEAM_ID in $config}"
  : "${HC_MACOS_SIGN_IDENTITY:?configure an existing keychain HC_MACOS_SIGN_IDENTITY in $config}"
  case "$HC_MACOS_BUNDLE_ID" in
    dev.herdr.core|*[!A-Za-z0-9.-]*|.*|*.|*..*) echo 'install-local: invalid or vendor bundle identifier' >&2; exit 1 ;;
  esac
  case "$HC_MACOS_BUNDLE_ID" in
    *.*) ;;
    *) echo 'install-local: bundle identifier must contain a dot' >&2; exit 1 ;;
  esac
  case "$HC_MACOS_TEAM_ID" in
    *[!A-Z0-9]*) echo 'install-local: invalid Apple team identifier' >&2; exit 1 ;;
  esac
  [ "${#HC_MACOS_TEAM_ID}" = 10 ] || { echo 'install-local: Apple team identifier must have 10 characters' >&2; exit 1; }
  [ "$HC_MACOS_SIGN_IDENTITY" != - ] || { echo 'install-local: ad-hoc source installation is not allowed' >&2; exit 1; }
  export HC_MACOS_BUNDLE_ID HC_MACOS_TEAM_ID HC_MACOS_SIGN_IDENTITY
fi
nice -n 10 cargo build --release
bin="${HC_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$bin"
if [ "$os" = Darwin ]; then
  app="target/release/Herdr Core.app"
  rm -rf "$app"
  sh scripts/mac-bundle target/release/herdr-core "$app"
  bash scripts/mac-sign "$app"
  "$app/Contents/MacOS/herdr-core" --hc-install-bundle "$app/Contents/MacOS/herdr-core" "$bin/herdr-core"
else
  install -m 755 target/release/herdr-core "$bin/herdr-core.new"
  mv -f "$bin/herdr-core.new" "$bin/herdr-core"
fi
[ -e "$bin/hc" ] || [ -L "$bin/hc" ] || ln -s herdr-core "$bin/hc"
sh scripts/restart-core.sh "$bin/herdr-core"
