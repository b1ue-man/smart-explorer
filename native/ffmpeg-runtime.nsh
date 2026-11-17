; Canonical FFmpeg runtime manifest of the Windows viewer (plan 2026-11-17
; "Windows-Bild" P8). Keep this file parseable by NSIS (installer.nsi),
; fetch-ffmpeg-runtime.ps1, fetch-ffmpeg-runtime.sh and the Rust loader
; (crates/engine/src/screen/os/windows/ffmpeg_runtime.rs). The archive is a
; month-end build of BtbN/FFmpeg-Builds (retained two years by that project's
; release policy), LGPL v3 shared: configured with --enable-version3, without
; --enable-gpl and --enable-nonfree. Only the three DLLs below are shipped.
!define FFMPEG_RELEASE "n8.1.3-27-g8dca1586f4"
!define FFMPEG_ZIP_FILENAME "ffmpeg-n8.1.3-27-g8dca1586f4-win64-lgpl-shared-8.1.zip"
!define FFMPEG_ZIP_URL "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-31-12-59/ffmpeg-n8.1.3-27-g8dca1586f4-win64-lgpl-shared-8.1.zip"
!define FFMPEG_ZIP_SIZE "80356594"
!define FFMPEG_ZIP_SHA256 "5cef7bc48e513f3cee7c881e8b2999d835541edb1846c3e89443a67f8f8556ab"
!define FFMPEG_ZIP_ROOT "ffmpeg-n8.1.3-27-g8dca1586f4-win64-lgpl-shared-8.1"
!define FFMPEG_AVUTIL_DLL "avutil-60.dll"
!define FFMPEG_AVUTIL_SIZE "3021824"
!define FFMPEG_AVUTIL_SHA256 "46ad06470515bf0f9a0f8f560470c725002c22decc6d7c2ae7ea836b5d941950"
!define FFMPEG_SWRESAMPLE_DLL "swresample-6.dll"
!define FFMPEG_SWRESAMPLE_SIZE "747008"
!define FFMPEG_SWRESAMPLE_SHA256 "790d6feb953be3118560afe6f5faafc1e254f947229699f40f95d70034eecf62"
!define FFMPEG_AVCODEC_DLL "avcodec-62.dll"
!define FFMPEG_AVCODEC_SIZE "91620864"
!define FFMPEG_AVCODEC_SHA256 "71ad4d3a1b158d88b4c00e8c2e1eded880e9a8a791a3bf9ba65fd053120470d9"
; LICENSE.txt of the archive (GNU LGPL version 3).
!define FFMPEG_LICENSE_SIZE "7651"
!define FFMPEG_LICENSE_SHA256 "da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768"
