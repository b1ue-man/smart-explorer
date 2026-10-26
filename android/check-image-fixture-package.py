#!/usr/bin/env python3
"""Inspect existing APK bytes on the remote runner; never build an artifact."""
import argparse
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import zipfile


BRIDGE = b"Lapp/smartexplorer/android/core/AndroidImageFixtureBridge;"
PREFIX = "Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_"
METHODS = {
    "captureOpen", "captureState", "captureConsumeStill", "captureViewerPullBuffer",
    "captureReleaseDecoderReader", "captureReleaseViewer", "captureRetire",
    "captureFinish", "loginOpen", "loginEnqueue", "loginState", "loginFinish",
}


def u32(data, offset):
    if offset < 0 or offset + 4 > len(data):
        raise ValueError("DEX field extends beyond the archive entry")
    return struct.unpack_from("<I", data, offset)[0]


def defined_classes(data):
    if len(data) < 112 or not data.startswith(b"dex\n") or data[7] != 0:
        raise ValueError("APK contains an invalid standard DEX entry")
    if u32(data, 32) != len(data) or u32(data, 40) != 0x12345678:
        raise ValueError("Unexpected DEX length or endian tag")
    string_count, strings = u32(data, 56), u32(data, 60)
    type_count, types = u32(data, 64), u32(data, 68)
    count, classes = u32(data, 96), u32(data, 100)
    for start, items, size in ((strings, string_count, 4), (types, type_count, 4),
                               (classes, count, 32)):
        if start + items * size > len(data):
            raise ValueError("DEX table extends beyond the archive entry")
    result = set()
    for index in range(count):
        type_index = u32(data, classes + index * 32)
        if type_index >= type_count:
            raise ValueError("DEX class has an invalid type index")
        string_index = u32(data, types + type_index * 4)
        if string_index >= string_count:
            raise ValueError("DEX type has an invalid descriptor index")
        cursor = u32(data, strings + string_index * 4)
        for _ in range(5):
            if cursor >= len(data):
                raise ValueError("Truncated DEX descriptor length")
            value = data[cursor]
            cursor += 1
            if value < 128:
                break
        else:
            raise ValueError("Invalid DEX descriptor length")
        end = data.find(b"\0", cursor)
        if end < 0:
            raise ValueError("Unterminated DEX descriptor")
        result.add(data[cursor:end])
    return result


def inspect_apk(path, nm):
    classes = set()
    symbols = {}
    with tempfile.TemporaryDirectory(prefix="android-image-package-") as temporary:
        with zipfile.ZipFile(path) as archive:
            names = archive.namelist()
            if len(names) != len(set(names)):
                raise ValueError("APK contains duplicate ZIP entries")
            for name in names:
                if name.startswith("classes") and name.endswith(".dex") and "/" not in name:
                    classes.update(defined_classes(archive.read(name)))
                if name.startswith("lib/") and name.endswith("/libsmart_explorer_android.so"):
                    # A fixed local basename avoids treating ZIP names as filesystem paths.
                    library = Path(temporary) / "bridge.so"
                    library.write_bytes(archive.read(name))
                    output = subprocess.run(
                        [nm, "--dynamic", "--defined-only", "--extern-only", str(library)],
                        check=True, capture_output=True, text=True, timeout=120,
                    ).stdout
                    symbols[name] = {line.split()[-1] for line in output.splitlines()
                                     if line.split() and PREFIX in line.split()[-1]}
    if not classes or not symbols:
        raise ValueError("APK lacks class definitions or native bridge libraries")
    return classes, symbols


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("debug", "release"))
    parser.add_argument("--apk", required=True, type=Path)
    parser.add_argument("--test-apk", type=Path)
    parser.add_argument("--nm", required=True)
    args = parser.parse_args()
    classes, symbols = inspect_apk(args.apk, args.nm)
    if args.mode == "debug":
        if BRIDGE not in classes:
            raise ValueError("Debug APK lacks the actual image fixture bridge class")
        expected = {PREFIX + method for method in METHODS}
        for library, exports in symbols.items():
            missing = sorted(expected - exports)
            if missing:
                raise ValueError(f"{library} lacks fixture JNI exports: {missing}")
        if args.test_apk is None:
            raise ValueError("Debug inspection requires the matching instrumentation APK")
        with zipfile.ZipFile(args.test_apk) as archive:
            if any(name.endswith(".so") for name in archive.namelist()):
                raise ValueError("Instrumentation APK must not contain its own native library")
    elif BRIDGE in classes or any(symbols.values()):
        raise ValueError("Release APK contains image fixture classes or JNI exports")
    print(json.dumps({"mode": args.mode, "apk": args.apk.name,
                      "libraries": sorted(symbols), "fixture_boundary": "verified"}))


if __name__ == "__main__":
    main()
