; Canonical Dokany installer dependency manifest.
; Keep this file parseable by NSIS, fetch-dokany-runtime.sh, and
; fetch-dokany-runtime.ps1. Values are pinned to the official GitHub release.
!define DOKANY_VERSION "2.3.1.1000"
; DokanVersion() reports the user-mode library API revision.
!define DOKANY_API_VERSION "231"
; DokanDriverVersion() reports the independent kernel protocol revision.
!define DOKANY_DRIVER_PROTOCOL_VERSION "400"
!define DOKANY_MSI_FILENAME "Dokan_x64.msi"
!define DOKANY_MSI_URL "https://github.com/dokan-dev/dokany/releases/download/v2.3.1.1000/Dokan_x64.msi"
!define DOKANY_MSI_SIZE "9269248"
!define DOKANY_MSI_SHA256 "69ff8cb37bfec3a75921c85ffd1c6370b50a9ec4ecef2cf3a009d488dcbf5465"
