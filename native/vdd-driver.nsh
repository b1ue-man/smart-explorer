; Canonical manifest of the Windows virtual display driver (plan 2026-11-17
; "Windows-Bild" P5). Keep this file parseable by NSIS (installer.nsi) and
; the Rust installer (crates/engine/src/screen/os/windows/virtual_display_pins.rs).
; The archive is the driver-only x64 package of VirtualDrivers/Virtual-Display-Driver
; release 25.7.23 (MIT licence). Its asset name says "x86", but its INF installs
; [Standard.NTamd64] and MttVDD.dll is a PE32+ AMD64 image (DriverVer 12/24/2024,
; 11.30.4.434, source tag 24.12.24). se never ships the binary: the elevated step
; downloads exactly this archive, checks size and SHA-256 of the archive and of
; each file, writes its own vdd_settings.xml and signs the catalog on the device
; with a certificate whose private key it deletes right afterwards.
!define VDD_RELEASE "25.7.23"
!define VDD_ZIP_FILENAME "VirtualDisplayDriver-x86.Driver.Only.zip"
!define VDD_ZIP_URL "https://github.com/VirtualDrivers/Virtual-Display-Driver/releases/download/25.7.23/VirtualDisplayDriver-x86.Driver.Only.zip"
!define VDD_ZIP_SIZE "132118"
!define VDD_ZIP_SHA256 "e24210692b442b39af763536330ce78b423f19342b7a7792c26de3944e418b3a"
!define VDD_ZIP_ROOT "VirtualDisplayDriver"
!define VDD_CAT_FILE "mttvdd.cat"
!define VDD_CAT_SIZE "13017"
!define VDD_CAT_SHA256 "08a0093fc9b2e32b287a6f8a77ca4de0a31830d29fc33d2b13a918dc859468f6"
!define VDD_DLL_FILE "MttVDD.dll"
!define VDD_DLL_SIZE "251640"
!define VDD_DLL_SHA256 "c9ca837f57a98fbd43bc416a7f535a95843626e7759eaf85cf0cd7ce334dbb05"
!define VDD_INF_FILE "MttVDD.inf"
!define VDD_INF_SIZE "4132"
!define VDD_INF_SHA256 "550d211fe481e74dfe3f9d724ed78be48b3a9113405965d683d9373e8d672f5d"
