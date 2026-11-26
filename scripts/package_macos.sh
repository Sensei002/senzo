#!/usr/bin/env bash
# Package the release binary into a self-contained SENZO.app for macOS.
#
# The app links the system libmpv (and whatever mpv pulls in), so the bundle
# has to copy every non-system dylib and re-point the load commands at the
# bundle itself. dylibbundler walks the dependency graph; the checks after it
# fail the build when anything still resolves outside the bundle, because such
# an archive launches only on a machine that happens to have the same paths.

set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/package_macos.sh --arch <arm64|x86_64> [options]

  --arch <arch>       Target architecture of the built binary (required).
  --version <version> App version for Info.plist (default: app/Cargo.toml).
  --tag <tag>         Release tag used in the archive name (default: v<version>).
  --out <dir>         Output directory (default: dist).
USAGE
}

arch=""
version=""
tag=""
out="dist"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch) arch="${2:?--arch needs a value}"; shift 2 ;;
        --version) version="${2:?--version needs a value}"; shift 2 ;;
        --tag) tag="${2:?--tag needs a value}"; shift 2 ;;
        --out) out="${2:?--out needs a value}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [[ -z "$arch" ]]; then
    echo "--arch is required" >&2
    usage >&2
    exit 2
fi

case "$arch" in
    arm64) target="aarch64-apple-darwin" ;;
    x86_64) target="x86_64-apple-darwin" ;;
    *) echo "unsupported architecture: $arch" >&2; exit 2 ;;
esac

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

binary="$root/target/release/senzo"
if [[ ! -f "$binary" ]]; then
    echo "missing release binary: $binary" >&2
    exit 1
fi

if [[ -z "$version" ]]; then
    version="$(grep -m1 '^version = ' app/Cargo.toml | cut -d'"' -f2)"
fi
if [[ -z "$version" ]]; then
    echo "could not determine the app version" >&2
    exit 1
fi
if [[ -z "$tag" ]]; then
    tag="v$version"
fi

for tool in sips iconutil dylibbundler otool install_name_tool codesign ditto hdiutil; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing build tool: $tool" >&2
        exit 1
    fi
done

echo "Packaging SENZO $version for $target"

rm -rf "$out"
mkdir -p "$out"
app="$out/SENZO.app"
contents="$app/Contents"
mkdir -p "$contents/MacOS" "$contents/Resources" "$contents/Frameworks"

cp "$binary" "$contents/MacOS/senzo"
chmod 755 "$contents/MacOS/senzo"

sed -e "s/__VERSION__/$version/g" assets/macos/Info.plist > "$contents/Info.plist"
printf 'APPL????' > "$contents/PkgInfo"

icon_source="app/ui/assets/images/app_icon.png"
iconset="$out/AppIcon.iconset"
rm -rf "$iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$icon_source" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z "$((size * 2))" "$((size * 2))" "$icon_source" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$contents/Resources/AppIcon.icns"
rm -rf "$iconset"

# Copy the runtime libraries (libmpv and its non-system dependencies) into the
# bundle and rewrite the load commands to point at @executable_path.
dylibbundler -b -cd -of \
    -x "$contents/MacOS/senzo" \
    -d "$contents/Frameworks" \
    -p "@executable_path/../Frameworks"

if ! otool -l "$contents/MacOS/senzo" | grep -q "@executable_path/../Frameworks"; then
    install_name_tool -add_rpath "@executable_path/../Frameworks" "$contents/MacOS/senzo"
fi

# Anything still linked from a Homebrew or MacPorts prefix ships only on
# machines that have the same prefix, so refuse to publish such a bundle.
stray=""
for candidate in "$contents/MacOS/senzo" "$contents"/Frameworks/*.dylib; do
    [[ -e "$candidate" ]] || continue
    while read -r dependency; do
        case "$dependency" in
            /opt/homebrew/*|/opt/local/*|/usr/local/*|/Users/*)
                stray="${stray}${candidate}: ${dependency}"$'\n'
                ;;
        esac
    done < <(otool -L "$candidate" | awk 'NR > 1 { print $1 }')
done
if [[ -n "$stray" ]]; then
    echo "the bundle still links libraries outside itself:" >&2
    printf '%s' "$stray" >&2
    exit 1
fi

codesign --force --deep --sign - "$app" >/dev/null
codesign --verify --deep --strict "$app"

# The disk image is the only macOS archive: mount it, drag SENZO onto the
# Applications alias, eject. Nothing else has to be unpacked by hand.
dmg="$out/senzo-${tag}-${target}.dmg"
stage="$out/dmg-stage"
rm -rf "$stage"
mkdir -p "$stage"
ditto "$app" "$stage/SENZO.app"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "SENZO" -srcfolder "$stage" -ov -format UDZO "$dmg" >/dev/null
rm -rf "$stage"
if [[ ! -s "$dmg" ]]; then
    echo "failed to build disk image: $dmg" >&2
    exit 1
fi

echo "Bundle: $app"
echo "Image:  $dmg ($(du -h "$dmg" | cut -f1))"
