#!/usr/bin/env bash
# Build target/installer/TimeTracker-<version>-x86_64.AppImage.
# usage: build-appimage.sh <version> <path-to-binary>
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <version> <path-to-binary>" >&2
  exit 2
fi
version="$1"
binary="$(realpath "$2")"

# appimagetool 1.9.1 (https://github.com/AppImage/appimagetool/releases/tag/1.9.1).
# Its runtime is static and does not need libfuse2.
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage"
APPIMAGETOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../../.." && pwd)"
cd "$root"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

curl --proto '=https' --tlsv1.2 -fsSL "$APPIMAGETOOL_URL" -o "$work/appimagetool"
echo "$APPIMAGETOOL_SHA256  $work/appimagetool" | sha256sum -c -
chmod +x "$work/appimagetool"

appdir="$work/AppDir"
mkdir -p "$appdir/usr/bin"
install -m 755 "$binary" "$appdir/usr/bin/timetracker"
install -m 644 "$here/timetracker.desktop" "$appdir/timetracker.desktop"
install -m 644 crates/tt-app/assets/app-256.png "$appdir/timetracker.png"
ln -s usr/bin/timetracker "$appdir/AppRun"

mkdir -p target/installer
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$work/appimagetool" "$appdir" \
  "target/installer/TimeTracker-$version-x86_64.AppImage"
