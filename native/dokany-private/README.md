# Private Dokany dependency recipe

This is Smart Explorer's modified **user-mode DLL**, not an official Dokany
release. It does not replace, rebuild, install or re-sign the shared System32
DLL, kernel driver or pinned official MSI. Non-drive features remain independent
of Dokany. The corrected DLL still uses API 231 and driver protocol 0x190/400.

## Source and correction

The source is the immutable Dokany commit
`f1d5de68ff459af94e309cfdd171e4b8ca2af4dd` (v2.3.1.1000), obtained from the URL,
exact ZIP length and SHA-256 in `recipe.json`. `batching.patch` makes three
bounded changes:

- Normalize batching/SingleThread worker options before sending them to the
  driver, and turn the batching mask into a Boolean comparison before its
  conversion to the one-byte `BOOLEAN` type.
- Add an aligned, atomic, private per-instance count of completed dispatches for
  second-or-later records from one kernel reply. No public Dokany structure or
  kernel protocol changes.
- Export `ULONGLONG DOKANAPI SmartExplorerBatchContinuationCountV1(DOKAN_HANDLE)`.
  Query only through the owning DLL while that filesystem handle remains open;
  this counter proves continuation dispatch, not successful remote writes.

Primary sources checked 2026-09-06: [pinned source](https://github.com/dokan-dev/dokany/tree/f1d5de68ff459af94e309cfdd171e4b8ca2af4dd),
[project settings](https://github.com/dokan-dev/dokany/blob/f1d5de68ff459af94e309cfdd171e4b8ca2af4dd/dokan/dokan.vcxproj),
[atomic alignment contract](https://learn.microsoft.com/en-us/windows/win32/api/winnt/nf-winnt-interlockedincrement64),
and [PE/COFF layout](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format).

## One preparation stage, exact-byte release reuse

The checked-in `../prepare-dokany-private.ps1` is a PowerShell 7.2+ entrypoint.
Never invoke its build mode on the Codex workstation. The configured remote
Windows runner sets `GITHUB_ACTIONS=true` or `SMART_EXPLORER_REMOTE_RUNNER=1`.
It needs VS 2022 C++ tools/v143, a complete Windows SDK at least 10.0.19041.0,
Git and HTTPS access to the pinned codeload URL. It does not run upstream build
scripts, build a driver, invoke WiX, install anything, or enable test signing.

The recipe deliberately selects and records installed VS 2022/v143, MSVC and
SDK versions rather than assuming upstream's older v142/19041 defaults exist.
MSBuild receives explicit platform/configuration/output paths and an empty user
property directory, with directory-level props/targets and auto-response files
disabled. The project uses the static multithreaded CRT. PE inspection rejects
non-AMD64 DLLs, changed/missing/forwarded exports, delay imports and dependencies
outside the recipe's reviewed system-DLL list.

Preparation is source/recipe reproducible, not a claim that different compiler
versions or signing runs produce bit-identical files. Exact bytes are retained
and reused instead of relying on such a claim.

```powershell
# Inside the single remote task-suite entrypoint, before its incremental Rust build:
$prepared = & ./native/prepare-dokany-private.ps1 -ArtifactDirectory $dependencyDir
$env:SMART_EXPLORER_DOKANY_DLL_DIR = $prepared.Directory
$env:SMART_EXPLORER_DOKANY_DLL_SHA256 = $prepared.DllSha256
```

An existing artifact directory is verified and reused, never overwritten or
silently rebuilt. A mismatched/partial directory fails. A new preparation keeps
its unique sibling `.dokany-private-stage.<guid>` directory as source/build
evidence; it prints this exact path on success or failure. These are remote
runner staging files, not release inputs to hand-promote after failure.

Output is exactly `dokan2.dll`, `manifest.json` and `corresponding-source.zip`.
After the complete mount suite approves those bytes, the main release owner
retains/commits that set under `native/assets/dokany-private/` and records the
approving suite run. This recipe script cannot declare its own output approved.

```powershell
# Terminal release preflight; works on a Windows or Linux PowerShell host:
& ./native/prepare-dokany-private.ps1 -VerifyOnly -RequireApproved `
    -ArtifactDirectory ./native/assets/dokany-private
```

`-VerifyOnly` performs no build/download/load/install. `-RequireApproved` also
requires the canonical approved directory and all three files tracked and
unchanged relative to HEAD. Optional `-ExpectedDllSha256` additionally binds any
verification/preparation result to a trusted externally supplied SHA. Release
automation must invoke verification before expensive builds and consume this
same approved set; it must not regenerate a different DLL.

## Embedding and manifest contract

`build.rs` delegates Windows-only preparation checks to
`build_support/private_dokany.rs`; it never compiles/downloads Dokany. Normal
Windows builds read the repository's approved directory. Only an absent
directory permits an official-only developer build with a Cargo warning.
Present but incomplete/invalid inputs fail. GNU resource compilation remains
unchanged, and GNU cross-build hosts require no MSBuild to embed approved bytes.

A bootstrap override requires **both** environment variables shown above. The
SHA must come from the trusted preparation stage, not an arbitrary adjacent
manifest. Both override and default paths verify source, recipe, patch, builder,
payload and corresponding-source hashes plus the recorded ABI/toolchain/PE
identity. The committed repository is the default input trust boundary;
`-RequireApproved` supplies the release tracking/cleanliness check.

The generated `OUT_DIR/private_dokany.rs` declares `pub(super)` constants:
`BUNDLED_DOKANY_BYTES: &[u8]`, `BUNDLED_DOKANY_SHA256: &str`, and
`BUNDLED_DOKANY_SOURCE: &str`, `BUNDLED_DOKANY_SOURCE_ARCHIVE: &[u8]`, and
`BUNDLED_DOKANY_SOURCE_SHA256: &str`. All five are empty for official-only
development builds. Otherwise the source string is the immutable upstream
commit above, and the verified corresponding-source ZIP and its SHA travel
inside the executable alongside the DLL. Runtime staging must make that source
and the license/notice texts accessible to portable and auto-updated users.

Manifest schema 1:

- Top-level `source_commit`, `source_archive_sha256`, `recipe_sha256`,
  `patch_sha256`, `builder_sha256`, `library_api`, `driver_protocol` and `schema`.
- `payload`: `file`, `size`, `sha256`, `machine`, sorted `imports`, sorted `exports`.
- `source_package`: `file`, `size`, `sha256`.
- `toolchain`: `vs_version`, `msvc_version`, `sdk_version`, `platform_toolset`,
  `runtime_library`.

Source ZIP/DLL/package hashes cover raw bytes. Recipe, patch and builder hashes
cover UTF-8 without BOM after CRLF-to-LF normalization, so checkout line endings
do not invalidate otherwise identical source inputs. Generated manifests use
UTF-8 without BOM. Hashes bind identity; an unauthenticated manifest by itself
does not prove how somebody built a DLL.

## Redistribution and modified-library rebuilding

Dokany's library is LGPL-3.0-or-later; see `LICENSE.LGPL-3.0.txt` (copied from
the pinned source) and `LICENSE.GPL-3.0.txt` (GNU's GPLv3 text). Preserve upstream
copyright notices. Smart Explorer's modifications are dated 2026-09-06 and are
identified by this recipe and patch. The corresponding-source ZIP contains
the complete patched upstream source, this recipe, the patch, preparation
script, this README and both license texts, before compiler outputs exist.

Distributors must deliver the notices/licenses and corresponding modified
source with the DLL, including when the DLL is embedded in an application.
Strict runtime byte pinning does not itself satisfy the replaceable-shared-
library alternative in LGPL section 4(d)(1). Provide the corresponding
application source/build materials and permitted recombination/relinking route
under section 4(d)(0), including instructions to rebuild the application with a
modified DLL and its newly prepared manifest/hash. Distribution integration is
owned by the complete release wrapper, not by this preparation script.

The source package may be unpacked to inspect/build the patched `dokan` project;
its `smartexplorer-build` folder contains the preparation script and inputs for
repeating the baseline download/patch/build. A modified-library developer can
update those source/patch pins in their application source and rebuild; normal
users are never asked to weaken DLL validation or machine security policy.
User-mode signing may be required by enterprise policy. It must occur before
the final payload hash and suite approval; this script does not assume access
to the upstream signing identity or alter driver-signing policy.
