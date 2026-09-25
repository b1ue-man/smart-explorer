#!/usr/bin/env bash
# Build, sign and verify the Smart Explorer Android release APK, or verify an
# already committed one, and record apk-metadata.json for the release wrapper.
#
# Callers: the build.yml android-release-apk job and, on a human-operated
# release host, native/publish-release-local.ps1 (bash on Linux, WSL on
# Windows). The top-level wrapper alone owns the version, feed and publication.
set -Eeuo pipefail
shopt -s inherit_errexit

usage() {
  cat <<'EOF'
Usage:
  android/build-release-apk.sh --check-env
  android/build-release-apk.sh [--expect-version X.Y.Z] --out DIR
  android/build-release-apk.sh [--expect-version X.Y.Z] --verify APK --out DIR

  --check-env       check the Android release toolchain; installs only the
                    pinned NDK (android/ndk-version) and the two Rust targets
  --out DIR         write smart-explorer-android.apk and apk-metadata.json;
                    DIR must be absent or empty
  --verify APK      verify an existing signed APK instead of building one
  --expect-version  fail unless native/Cargo.toml carries this version

Build signing: ANDROID_KEYSTORE_FILE, ANDROID_KEYSTORE_PASSWORD,
ANDROID_KEY_ALIAS, ANDROID_KEY_PASSWORD. SDK: ANDROID_HOME or ANDROID_SDK_ROOT.
EOF
}

die() {
  echo "build-release-apk: $*" >&2
  exit 1
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
native_dir="$repo_root/native"
apk_name="smart-explorer-android.apk"
metadata_name="apk-metadata.json"
bridge_package="smart_explorer_android"
bridge_library="libsmart_explorer_android.so"
android_platform=30
abis=(arm64-v8a x86_64)
rust_targets=(aarch64-linux-android x86_64-linux-android)
jni_libs="$script_dir/app/src/main/jniLibs"
apk_output_dir="$script_dir/app/build/outputs/apk/release"

mode=build
check_env_requested=0
out_dir=""
verify_source=""
expect_version=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --check-env)
      check_env_requested=1
      ;;
    --out)
      [ "$#" -ge 2 ] || die "--out needs a directory"
      out_dir="$2"
      shift
      ;;
    --verify)
      [ "$#" -ge 2 ] || die "--verify needs an APK path"
      verify_source="$2"
      mode=verify
      shift
      ;;
    --expect-version)
      [ "$#" -ge 2 ] || die "--expect-version needs X.Y.Z"
      expect_version="$2"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  shift
done
if [ "$check_env_requested" = 1 ]; then
  if [ -n "$out_dir" ] || [ -n "$verify_source" ] || [ -n "$expect_version" ]; then
    die "--check-env takes no other option"
  fi
  mode=check-env
elif [ -z "$out_dir" ]; then
  die "--out is required"
fi

# Release profile pins shared with native/publish-feed.sh; the job count stays
# an operator/runner resource choice (the complete release wrapper sets 1).
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_RELEASE_LTO=thin
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=8
export CARGO_PROFILE_RELEASE_DEBUG=0
export CARGO_TERM_COLOR=never

require_tools() {
  local tool
  for tool in "$@"; do
    command -v "$tool" >/dev/null 2>&1 || die "required tool missing: $tool"
  done
}

cargo_version() {
  local version
  version="$(sed -nE '/^version[[:space:]]*=[[:space:]]*"/{s/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p;q}' \
    "$native_dir/Cargo.toml")"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
    die "native/Cargo.toml has no stable X.Y.Z package version: '$version'"
  printf '%s\n' "$version"
}

# versionCode = major*1000000 + minor*1000 + patch, as in android/app/build.gradle.kts
# and Get-PublicationAndroidVersionCode in native/release-publication.ps1.
version_code_for() {
  local major minor patch code
  IFS=. read -r major minor patch <<<"$1"
  [ "${#major}" -le 4 ] || die "Android versionCode cannot encode major version '$major'"
  if ((10#$minor >= 1000 || 10#$patch >= 1000)); then
    die "Android versionCode requires minor and patch below 1000; got '$1'"
  fi
  code=$((10#$major * 1000000 + 10#$minor * 1000 + 10#$patch))
  if ((code <= 0 || code > 2100000000)); then
    die "Android versionCode $code for '$1' is outside 1..2100000000"
  fi
  printf '%s\n' "$code"
}

expected_cert() {
  local file="$script_dir/release-cert.sha256" value
  [ -s "$file" ] || die "android/release-cert.sha256 is missing"
  value="$(awk 'NR == 1 { print tolower($1) }' "$file")"
  [[ "$value" =~ ^[0-9a-f]{64}$ ]] ||
    die "android/release-cert.sha256 must start with one SHA-256 certificate fingerprint"
  printf '%s\n' "$value"
}

resolve_sdk() {
  sdk_root="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [ -z "$sdk_root" ] || [ ! -d "$sdk_root" ]; then
    die "set ANDROID_HOME (or ANDROID_SDK_ROOT) to the Android SDK"
  fi
  export ANDROID_HOME="$sdk_root"
  sdkmanager="$sdk_root/cmdline-tools/latest/bin/sdkmanager"
  if [ ! -x "$sdkmanager" ]; then
    sdkmanager="$(command -v sdkmanager || true)"
  fi
}

ensure_ndk() {
  local ndk_version
  [ -s "$script_dir/ndk-version" ] || die "android/ndk-version is missing"
  ndk_version="$(tr -d '[:space:]' <"$script_dir/ndk-version")"
  [[ "$ndk_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
    die "android/ndk-version must contain one NDK version, got '$ndk_version'"
  ndk_home="$sdk_root/ndk/$ndk_version"
  if [ ! -f "$ndk_home/source.properties" ]; then
    [ -n "$sdkmanager" ] || die "NDK $ndk_version is missing and sdkmanager was not found"
    echo "Installing the pinned Android NDK $ndk_version ..."
    # yes(1) ends with SIGPIPE once sdkmanager exits; the pipeline status is
    # sdkmanager's own, which errexit still enforces.
    set +o pipefail
    yes | "$sdkmanager" --licenses >/dev/null
    yes | "$sdkmanager" --install "ndk;$ndk_version"
    set -o pipefail
  fi
  if [ ! -f "$ndk_home/source.properties" ] ||
    ! grep -Fq "$ndk_version" "$ndk_home/source.properties"; then
    die "NDK $ndk_version is not installed at $ndk_home"
  fi
  llvm_objdump="$ndk_home/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-objdump"
  [ -x "$llvm_objdump" ] || die "NDK $ndk_version lacks $llvm_objdump"
  export ANDROID_NDK_HOME="$ndk_home"
}

resolve_build_tools() {
  local version dir
  aapt2=""
  apksigner=""
  [ -d "$sdk_root/build-tools" ] || die "Android SDK build-tools are missing under $sdk_root"
  while IFS= read -r version; do
    dir="$sdk_root/build-tools/$version"
    if [ -x "$dir/aapt2" ] && [ -x "$dir/apksigner" ]; then
      aapt2="$dir/aapt2"
      apksigner="$dir/apksigner"
      return 0
    fi
  done < <(find "$sdk_root/build-tools" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort -rV)
  die "no Android SDK build-tools with aapt2 and apksigner under $sdk_root/build-tools"
}

ensure_rust_targets() {
  local installed target
  installed="$(rustup target list --installed)"
  for target in "${rust_targets[@]}"; do
    if ! grep -Fxq "$target" <<<"$installed"; then
      rustup target add "$target"
    fi
  done
}

require_signing() {
  local name
  for name in ANDROID_KEYSTORE_FILE ANDROID_KEYSTORE_PASSWORD ANDROID_KEY_ALIAS ANDROID_KEY_PASSWORD; do
    [ -n "${!name:-}" ] || die "release signing requires $name"
  done
  if [ ! -f "$ANDROID_KEYSTORE_FILE" ] || [ ! -r "$ANDROID_KEYSTORE_FILE" ]; then
    die "ANDROID_KEYSTORE_FILE is not a readable file"
  fi
}

check_build_env() {
  local version
  require_tools cargo rustup cargo-ndk java python3 sha256sum find sort
  [ -f "$script_dir/gradlew" ] || die "android/gradlew is missing"
  expected_cert >/dev/null
  version="$(cargo_version)"
  version_code_for "$version" >/dev/null
  resolve_sdk
  ensure_ndk
  resolve_build_tools
  ensure_rust_targets
  require_signing
}

rustls_maven_dir() {
  local metadata maven
  metadata="$(cd "$native_dir" && cargo metadata --locked --format-version 1 \
    --filter-platform aarch64-linux-android --manifest-path "$native_dir/Cargo.toml")"
  maven="$(python3 -c '
import json, os, sys
data = json.load(sys.stdin)
paths = sorted({p["manifest_path"] for p in data["packages"]
                if p["name"] == "rustls-platform-verifier-android"})
if len(paths) != 1:
    sys.exit("expected one rustls-platform-verifier-android package, found %d" % len(paths))
print(os.path.join(os.path.dirname(paths[0]), "maven"))
' <<<"$metadata")"
  [ -d "$maven/rustls/rustls-platform-verifier" ] ||
    die "rustls-platform-verifier Maven repository missing at $maven"
  printf '%s\n' "$maven"
}

assert_16k_aligned() {
  local library=$1 aligns align
  aligns="$("$llvm_objdump" -p "$library" |
    awk '$1 == "LOAD" { for (i = 1; i < NF; i++) if ($i == "align") print $(i + 1) }')"
  [ -n "$aligns" ] || die "no LOAD segments reported for $library"
  while IFS= read -r align; do
    [[ "$align" =~ ^2\*\*([0-9]+)$ ]] || die "unexpected LOAD alignment '$align' in $library"
    ((BASH_REMATCH[1] >= 14)) || die "$library is not 16 KB aligned (LOAD align $align)"
  done <<<"$aligns"
}

build_apk() {
  local maven abi library ndk_args=()
  maven="$(rustls_maven_dir)"
  for abi in "${abis[@]}"; do
    ndk_args+=(-t "$abi")
    rm -f -- "$jni_libs/$abi/$bridge_library"
  done
  (
    cd "$native_dir"
    cargo ndk "${ndk_args[@]}" --platform "$android_platform" -o "$jni_libs" \
      build --release --locked -p "$bridge_package"
  )
  for abi in "${abis[@]}"; do
    library="$jni_libs/$abi/$bridge_library"
    [ -s "$library" ] || die "cargo-ndk did not produce $library"
    assert_16k_aligned "$library"
  done
  rm -rf -- "$apk_output_dir"
  (
    cd "$script_dir"
    sh ./gradlew --no-daemon --console=plain :app:assembleRelease "-PrustlsVerifierMaven=$maven"
  )
  local apks=()
  mapfile -t apks < <(find "$apk_output_dir" -maxdepth 1 -type f -name '*.apk' | sort)
  [ "${#apks[@]}" -eq 1 ] || die "expected one release APK in $apk_output_dir, found ${#apks[@]}"
  case "${apks[0]}" in
    *-unsigned.apk) die "Gradle produced an unsigned APK; check the release signing configuration" ;;
  esac
  built_apk="${apks[0]}"
}

verify_apk() {
  local apk=$1 version=$2 code=$3 cert=$4
  local badging package_line apk_code apk_version certs digests
  local code_pattern="versionCode='([0-9]+)'" name_pattern="versionName='([^']*)'"
  badging="$("$aapt2" dump badging "$apk")" || die "aapt2 could not read $apk"
  package_line="$(grep -m1 '^package: ' <<<"$badging")" || die "aapt2 reported no package line for $apk"
  [[ "$package_line" =~ $code_pattern ]] || die "APK has no versionCode: $package_line"
  apk_code="${BASH_REMATCH[1]}"
  [[ "$package_line" =~ $name_pattern ]] || die "APK has no versionName: $package_line"
  apk_version="${BASH_REMATCH[1]}"
  [ "$apk_version" = "$version" ] || die "APK versionName '$apk_version' is not '$version'"
  [ "$apk_code" = "$code" ] || die "APK versionCode '$apk_code' is not '$code'"
  certs="$("$apksigner" verify --print-certs "$apk" 2>&1)" || {
    printf '%s\n' "$certs" >&2
    die "apksigner rejected $apk"
  }
  # Signer lines read "Signer #1 certificate …" or, with a v3.1 block, "Signer (minSdkVersion=…,
  # maxSdkVersion=…) certificate …"; source-stamp and lineage certificates are not signers.
  digests="$(sed -nE '/^Source Stamp Signer /d; / in lineage certificate /d;
    s/^Signer [^:]* certificate SHA-256 digest: *([0-9A-Fa-f]{64})[[:space:]]*$/\1/p' <<<"$certs" |
    tr 'A-F' 'a-f' | sort -u)"
  if [ -z "$digests" ]; then
    printf 'apksigner --print-certs output:\n%s\n' "$certs" >&2
    die "apksigner reported no signer certificate for $apk"
  fi
  [ "$(wc -l <<<"$digests")" -eq 1 ] || die "APK is signed by more than one certificate"
  [ "$digests" = "$cert" ] ||
    die "APK signer $digests does not match android/release-cert.sha256 ($cert)"
  python3 - "$apk" "$bridge_library" "${abis[@]}" <<'PY'
import sys
import zipfile

apk, library, *abis = sys.argv[1:]
with zipfile.ZipFile(apk) as archive:
    names = set(archive.namelist())
missing = [f"lib/{abi}/{library}" for abi in abis if f"lib/{abi}/{library}" not in names]
if missing:
    sys.exit("APK lacks native libraries: " + ", ".join(missing))
PY
}

# Copies the APK into the output directory first and verifies that copy, so
# the recorded metadata describes exactly the handed-over bytes.
publish_verified() {
  local source=$1 version=$2 code=$3 cert=$4 partial sha
  if [ -e "$out_dir" ]; then
    [ -d "$out_dir" ] || die "--out exists and is not a directory: $out_dir"
    [ -z "$(find "$out_dir" -mindepth 1 -maxdepth 1 -print -quit)" ] ||
      die "--out directory must be empty: $out_dir"
  fi
  mkdir -p -- "$out_dir"
  # Keep the .apk suffix on the unverified copy for the SDK tools.
  partial="$out_dir/.partial-$apk_name"
  cp -- "$source" "$partial"
  verify_apk "$partial" "$version" "$code" "$cert"
  sha="$(sha256sum "$partial" | awk '{ print $1 }')"
  [[ "$sha" =~ ^[0-9a-f]{64}$ ]] || die "could not hash $partial"
  mv -- "$partial" "$out_dir/$apk_name"
  python3 - "$out_dir/.$metadata_name.partial" "$version" "$code" "$sha" "$cert" <<'PY'
import json
import sys

path, version, code, sha, cert = sys.argv[1:]
with open(path, "w", encoding="utf-8", newline="\n") as handle:
    json.dump(
        {"version": version, "versionCode": int(code), "sha256": sha, "certSha256": cert},
        handle,
        indent=2,
    )
    handle.write("\n")
PY
  mv -- "$out_dir/.$metadata_name.partial" "$out_dir/$metadata_name"
  echo "Android release APK verified: v$version (versionCode $code), SHA-256 $sha"
}

if [ "$mode" = check-env ]; then
  check_build_env
  echo "Android release environment OK: NDK $ANDROID_NDK_HOME, aapt2 $aapt2, signing inputs present."
  exit 0
fi

version="$(cargo_version)"
if [ -n "$expect_version" ] && [ "$expect_version" != "$version" ]; then
  die "native/Cargo.toml is $version, expected $expect_version"
fi
code="$(version_code_for "$version")"
cert="$(expected_cert)"

if [ "$mode" = verify ]; then
  require_tools java python3 sha256sum find sort
  [ -s "$verify_source" ] || die "APK to verify is missing or empty: $verify_source"
  if [ -f "$verify_source.sha256" ]; then
    (
      cd "$(dirname "$verify_source")"
      sha256sum --check --strict "$(basename "$verify_source").sha256"
    ) || die "committed sidecar does not bind $verify_source"
  fi
  resolve_sdk
  resolve_build_tools
  publish_verified "$verify_source" "$version" "$code" "$cert"
  exit 0
fi

check_build_env
build_apk
publish_verified "$built_apk" "$version" "$code" "$cert"
