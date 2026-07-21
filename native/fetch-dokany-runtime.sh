#!/usr/bin/env bash
# Fetch the exact Dokany MSI used by installer.nsi into the ignored build cache.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
manifest="$script_dir/dokany-runtime.nsh"
output=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --output)
      [ "$#" -ge 2 ] || { echo "--output requires a path" >&2; exit 2; }
      output=$2
      shift
      ;;
    *)
      echo "Unknown argument: $1" >&2
      echo "Usage: native/fetch-dokany-runtime.sh [--output PATH]" >&2
      exit 2
      ;;
  esac
  shift
done

for tool in curl sha256sum wc mktemp mv chmod mkdir dirname sed awk tr unlink; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "Required Dokany fetch tool missing: $tool" >&2
    exit 1
  }
done
test -f "$manifest" || { echo "Dokany manifest missing: $manifest" >&2; exit 1; }

manifest_value() {
  local key=$1
  local value
  value="$(sed -nE "s/^!define[[:space:]]+$key[[:space:]]+\"([^\"]+)\"[[:space:]]*$/\\1/p" "$manifest")"
  [ "$(printf '%s\n' "$value" | awk 'NF { count++ } END { print count + 0 }')" = "1" ] || {
    echo "Dokany manifest must define $key exactly once." >&2
    exit 1
  }
  printf '%s' "$value"
}

version="$(manifest_value DOKANY_VERSION)"
api_version="$(manifest_value DOKANY_API_VERSION)"
filename="$(manifest_value DOKANY_MSI_FILENAME)"
url="$(manifest_value DOKANY_MSI_URL)"
expected_size="$(manifest_value DOKANY_MSI_SIZE)"
expected_sha256="$(manifest_value DOKANY_MSI_SHA256)"

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "Invalid Dokany version in manifest: $version" >&2
  exit 1
}
[[ "$api_version" =~ ^[1-9][0-9]*$ ]] || {
  echo "Invalid Dokany API version in manifest: $api_version" >&2
  exit 1
}
[[ "$filename" =~ ^[A-Za-z0-9._-]+\.msi$ ]] || {
  echo "Unsafe Dokany MSI filename in manifest: $filename" >&2
  exit 1
}
[[ "$url" == "https://github.com/dokan-dev/dokany/releases/download/v$version/$filename" ]] || {
  echo "Dokany MSI URL is not the pinned official release asset." >&2
  exit 1
}
[[ "$expected_size" =~ ^[1-9][0-9]*$ ]] || {
  echo "Invalid Dokany MSI size in manifest: $expected_size" >&2
  exit 1
}
[[ "$expected_sha256" =~ ^[0-9a-f]{64}$ ]] || {
  echo "Invalid Dokany MSI SHA-256 in manifest." >&2
  exit 1
}

if [ -z "$output" ]; then
  output="$script_dir/target/installer-dependencies/$version/$filename"
fi
output_parent="$(dirname "$output")"
mkdir -p "$output_parent"

verify_msi() {
  local path=$1
  [ -f "$path" ] || return 1
  [ "$(wc -c < "$path" | tr -d '[:space:]')" = "$expected_size" ] || return 1
  [ "$(sha256sum "$path" | awk '{print $1}')" = "$expected_sha256" ] || return 1
}

if verify_msi "$output"; then
  printf '%s\n' "$output"
  exit 0
fi

temporary="$(mktemp "$output_parent/.${filename}.partial.XXXXXX")"
cleanup() {
  if [ -n "${temporary:-}" ] && [ -f "$temporary" ]; then
    # A partial file is private build state and never a release input.
    unlink "$temporary"
  fi
}
trap cleanup EXIT

curl --proto '=https' --tlsv1.2 --fail --location --retry 4 --retry-all-errors \
  --output "$temporary" "$url"
verify_msi "$temporary" || {
  echo "Downloaded Dokany MSI failed pinned size/SHA-256 verification." >&2
  exit 1
}
chmod 0644 "$temporary"
mv -f -- "$temporary" "$output"
temporary=""
verify_msi "$output" || {
  echo "Promoted Dokany MSI failed verification: $output" >&2
  exit 1
}
printf '%s\n' "$output"
