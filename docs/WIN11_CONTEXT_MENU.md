# Windows 11 modern context-menu entry

Status: **implementation done + compile-verified; blocked only on code signing.**

This is the last "Later" roadmap item. The goal is the *main* (modern) Win11
right-click menu showing **"In Smart Explorer öffnen"** without the user having
to click *"Show more options"*.

## Why the legacy verb isn't enough

`shell_register.rs` already installs a legacy registry verb. On Windows 11 a
legacy verb only ever appears under **"Show more options"** (the classic menu).
The modern menu is closed to registry verbs — it only renders entries provided
by an **`IExplorerCommand` COM handler that is declared by a packaged identity**
(sparse/MSIX). See `docs/GOTCHAS.md` for the live test that disproved every
registry-only shortcut.

## What is implemented here (the feasible half)

`native/explorer-command/` is a **standalone Rust cdylib** (separate from the
app — the app does not depend on it):

- `src/lib.rs` — the in-proc COM server:
  - `OpenCommand` implements `IExplorerCommand` (title = "In Smart Explorer
    öffnen"; `Invoke` launches `Smart Explorer.exe` — resolved as a sibling of
    the DLL — with the selected path).
  - `Factory` implements `IClassFactory`; `DllGetClassObject` / `DllCanUnloadNow`
    / `DllMain` are exported.
  - CLSID `{7F3B1E20-9C4A-4D8E-A1B2-3C4D5E6F7081}`.
- `AppxManifest.xml` — a **sparse package** manifest that wires the CLSID into
  the modern menu (`windows.comServer` + `windows.fileExplorerContextMenus` on
  Directory, Directory\Background and Drive) while leaving the app unpackaged
  (per-user install stays under `%LOCALAPPDATA%\Programs\Smart Explorer`).
- `build-dll.sh` — cross-compiles the DLL and checks its exports.

Build + verification done in CI/this repo:

```bash
native/explorer-command/build-dll.sh
# -> target/x86_64-pc-windows-gnu/release/smart_explorer_command.dll  (PE32+ DLL)
# -> exports: DllCanUnloadNow, DllGetClassObject
```

The DLL compiles and links cleanly for `x86_64-pc-windows-gnu` and exports the
correct COM entry points. **This proves the COM half is feasible** (the part
GOTCHAS flagged as doable).

## The wall: signing (cannot be crossed without a trusted cert)

A sparse package **must be signed**, and Windows must **trust the signing cert**,
or it refuses to register the package — so the modern menu never appears. The
options:

1. **Self-signed cert** → every machine must first import the cert into
   *Trusted People / Trusted Root* (admin). A non-starter for an unsigned,
   per-user, no-admin app.
2. **Purchased OV/EV code-signing cert** or **Azure Trusted Signing** → works
   everywhere, costs money / an enrolled account.

Because the project ships unsigned and admin-free, this stays **deferred at the
signing step** — exactly as GOTCHAS concluded. Everything *up to* signing is
done and in the repo.

## Finishing on a machine with a trusted cert

1. Build the DLL (`build-dll.sh`) and copy it next to the manifest in a staging
   folder together with `Smart Explorer.exe` and the `Assets\*` logos.
2. Fill the manifest placeholders: `Publisher` = your cert subject (exact),
   `uap10:Executable` = the installed exe path, `Version`.
3. Package + sign (Windows SDK):
   ```powershell
   makeappx pack /d staging /p SmartExplorer.ContextMenu.msix
   signtool sign /fd SHA256 /a /f cert.pfx /p <pwd> SmartExplorer.ContextMenu.msix
   ```
   (or register the sparse package against the external app folder with
   `Add-AppxPackage -Register AppxManifest.xml -ExternalLocation <dir>` after
   signing).
4. Install: `Add-AppxPackage SmartExplorer.ContextMenu.msix`. The verb then
   appears in the **main** Win11 context menu.

Validate the manifest on Windows (`makeappx`/App Certification Kit) — it was
authored to schema but not validated in the Linux build sandbox.
