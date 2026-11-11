#!/usr/bin/env bash
# Build target/installer/TimeTracker-<version>-macos-universal.dmg (ad-hoc signed, not notarized).
# usage: build-dmg.sh <version> <arm64-binary> <x86_64-binary>
# Runs on macOS only (lipo, plutil, codesign, hdiutil); keep to bash 3.2 and BSD tools.
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <version> <arm64-binary> <x86_64-binary>" >&2
  exit 2
fi
version="$1"
arm64_bin="$2"
x86_64_bin="$3"
if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'; then
  echo "error: version '$version' must look like X.Y.Z or X.Y.Z-suffix" >&2
  exit 2
fi
# CFBundleShortVersionString accepts only X.Y.Z; the full version goes into the .dmg name.
short="${version%%-*}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../../.." && pwd)"
cd "$root"
mkdir -p target/installer
out="$root/target/installer/TimeTracker-$version-macos-universal.dmg"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

dmg_dir="$work/dmg"
app="$dmg_dir/TimeTracker.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

lipo -create -output "$app/Contents/MacOS/TimeTracker" "$arm64_bin" "$x86_64_bin"
archs="$(lipo -archs "$app/Contents/MacOS/TimeTracker")"
echo "architectures: $archs"
case " $archs " in
  *" x86_64 "*) ;;
  *) echo "error: x86_64 slice is missing" >&2; exit 1 ;;
esac
case " $archs " in
  *" arm64 "*) ;;
  *) echo "error: arm64 slice is missing" >&2; exit 1 ;;
esac

sed "s/@VERSION@/$short/g" "$here/Info.plist" > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
cp crates/tt-app/assets/app.icns "$app/Contents/Resources/TimeTracker.icns"

codesign --force --sign - --timestamp=none "$app"
codesign --verify --strict --verbose=2 "$app"

ln -s /Applications "$dmg_dir/Applications"
hdiutil create -volname TimeTracker -srcfolder "$dmg_dir" -fs HFS+ -format UDZO -ov "$out"

# Minimum macOS per slice, for the CI log only.
for arch in arm64 x86_64; do
  echo "== $arch =="
  lipo -thin "$arch" "$app/Contents/MacOS/TimeTracker" -output "$work/thin-$arch"
  otool -l "$work/thin-$arch" | grep -A4 LC_BUILD_VERSION || true
done
ls -l "$out"
