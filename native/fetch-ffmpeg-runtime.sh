#!/usr/bin/env bash
# Fetch the pinned FFmpeg runtime of the Windows viewer (plan 2026-11-17 P8)
# into the ignored build cache, as fetch-ffmpeg-runtime.ps1 does: the archive
# of ffmpeg-runtime.nsh (exact size and SHA-256), then its three DLLs and the
# licence, each checked against its pin. Prints the directory with the files.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
manifest="$script_dir/ffmpeg-runtime.nsh"
output=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --output)
      [ "$#" -ge 2 ] || { echo "--output requires a directory" >&2; exit 2; }
      output=$2
      shift
      ;;
    *)
      echo "Usage: native/fetch-ffmpeg-runtime.sh [--output DIR]" >&2
      exit 2
      ;;
  esac
  shift
done

for tool in curl sha256sum stat unzip mktemp mv mkdir; do
  command -v "$tool" >/dev/null 2>&1 || { echo "Required FFmpeg fetch tool missing: $tool" >&2; exit 1; }
done
test -f "$manifest" || { echo "FFmpeg manifest missing: $manifest" >&2; exit 1; }

manifest_value() {
  local value
  value="$(sed -nE "s/^!define[[:space:]]+$1[[:space:]]+\"([^\"]+)\"[[:space:]]*$/\\1/p" "$manifest")"
  [ "$(printf '%s\n' "$value" | awk 'NF { count++ } END { print count + 0 }')" = "1" ] || {
    echo "FFmpeg manifest must define $1 exactly once." >&2
    exit 1
  }
  printf '%s' "$value"
}

matches_pin() { # path size sha256
  [ -f "$1" ] && [ "$(stat -c %s "$1")" = "$2" ] && [ "$(sha256sum "$1" | awk '{print $1}')" = "$3" ]
}

release="$(manifest_value FFMPEG_RELEASE)"
zip_name="$(manifest_value FFMPEG_ZIP_FILENAME)"
url="$(manifest_value FFMPEG_ZIP_URL)"
root="$(manifest_value FFMPEG_ZIP_ROOT)"
[[ "$zip_name" =~ ^[A-Za-z0-9._-]+\.zip$ && "$root" == "${zip_name%.zip}" ]] || {
  echo "Unsafe FFmpeg archive name in manifest: $zip_name" >&2
  exit 1
}
[[ "$url" == https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-*/"$zip_name" ]] || {
  echo "FFmpeg archive URL is not a pinned BtbN release asset." >&2
  exit 1
}
cache="$script_dir/target/installer-dependencies/ffmpeg-$release"
output="${output:-$cache/files}"
mkdir -p "$cache" "$output"
names=("$(manifest_value FFMPEG_AVUTIL_DLL)" "$(manifest_value FFMPEG_SWRESAMPLE_DLL)" "$(manifest_value FFMPEG_AVCODEC_DLL)" LICENSE.txt)
prefixes=(FFMPEG_AVUTIL FFMPEG_SWRESAMPLE FFMPEG_AVCODEC FFMPEG_LICENSE)

complete=1
for i in 0 1 2 3; do
  matches_pin "$output/${names[$i]}" "$(manifest_value "${prefixes[$i]}_SIZE")" \
    "$(manifest_value "${prefixes[$i]}_SHA256")" || complete=0
done
if [ "$complete" = "1" ]; then
  printf '%s\n' "$output"
  exit 0
fi

archive="$cache/$zip_name"
if ! matches_pin "$archive" "$(manifest_value FFMPEG_ZIP_SIZE)" "$(manifest_value FFMPEG_ZIP_SHA256)"; then
  partial="$(mktemp "$cache/.ffmpeg.partial.XXXXXX")"
  trap 'rm -f "$partial"' EXIT
  curl --fail --location --proto '=https' --tlsv1.2 --retry 3 --silent --show-error -o "$partial" "$url"
  matches_pin "$partial" "$(manifest_value FFMPEG_ZIP_SIZE)" "$(manifest_value FFMPEG_ZIP_SHA256)" || {
    echo "Downloaded FFmpeg archive does not match its pinned size and SHA-256." >&2
    exit 1
  }
  mv -f "$partial" "$archive"
fi

for i in 0 1 2 3; do
  name="${names[$i]}"
  inner="$root/bin/$name"
  [ "$name" = "LICENSE.txt" ] && inner="$root/LICENSE.txt"
  unzip -p "$archive" "$inner" > "$output/$name.partial"
  matches_pin "$output/$name.partial" "$(manifest_value "${prefixes[$i]}_SIZE")" \
    "$(manifest_value "${prefixes[$i]}_SHA256")" || {
    rm -f "$output/$name.partial"
    echo "$name from the FFmpeg archive does not match its pin." >&2
    exit 1
  }
  mv -f "$output/$name.partial" "$output/$name"
done
printf '%s\n' "$output"
