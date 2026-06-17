# Microsoft Cloud Files API (CfAPI) & OneDrive Files On-Demand — design intent and limits

Research for "Smart Explorer" VFS decision: (A) on-demand placeholder VFS (Windows Cloud Files
API / OneDrive Files On-Demand) vs (B) download-a-real-copy-and-save-back. This file documents the
**designers' own documented rationale and limitations** for CfAPI, with verbatim quotes and URLs.

Bottom line up front: CfAPI is designed for a **continuously running sync ENGINE that owns a folder
tree** (OneDrive, Dropbox, etc.) — a registered, always-available provider that services platform
callbacks. It is explicitly *not* shaped for a transient "open one remote file, edit it, save it
back" action performed by an app that is not an always-running sync daemon.

---

## 1. What CfAPI is FOR (intended use case / who should build one)

CfAPI exists to let a **sync engine** present remote files inside the Windows file system and File
Explorer. Microsoft defines the target workload explicitly:

> "A sync engine is a service that syncs files, typically between a remote host and a local client.
> Sync engines on Windows often present those files to the user through the Windows file system and
> File Explorer."
> — Build a Cloud Sync Engine that Supports Placeholder Files
> https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine

> "Windows 10 version 1709 (also called the Fall Creators Update) introduced the *cloud files API*.
> This API is a new platform that formalizes support for sync engines."
> — same page

> "Starting in Windows 10, version 1709, Windows provides the *cloud files API*. This API consists
> of several native Win32 and WinRT APIs that formalize support for cloud sync engines, and handles
> tasks such as creating and managing placeholder files and directories. Users of this API are
> typically sync providers and to some extent, Windows applications."
> — Cloud Sync Engines (portal)
> https://learn.microsoft.com/en-us/windows/win32/cfapi/cloud-files-api-portal

The API has two cooperating components, both of which a provider must use:

> "- **Cloud Filter API**: This native Win32 API provides functionality at the boundary between the
>   user mode and the file system. This API handles the creation and management of placeholder files
>   and directories.
> - **Windows.Storage.Provider namespace**: This WinRT API enables applications to configure the
>   cloud storage provider and register the sync root with the operating system."
> — Build a Cloud Sync Engine
> https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine

**Who should build one:** a cloud storage *provider* shipping a long-lived sync client (the OneDrive/
Dropbox/Google Drive desktop-app archetype) that wants its whole remote namespace to appear natively
in Explorer with placeholders, status overlays, context menus, and on-demand hydration. It must be a
**desktop app**, not UWP:

> "The cloud files API does not currently support implementing cloud sync engines in UWP apps. Cloud
> sync engines must be implemented in desktop apps."
> — same page

OneDrive Files On-Demand is the canonical consumer of this API and frames the user-facing value:

> "OneDrive Files On-Demand helps you access all the files in your cloud storage in OneDrive without
> having to download them and use storage space on your computer."
> — Save disk space with OneDrive Files On-Demand for Windows (Microsoft Support)
> https://support.microsoft.com/en-us/office/save-disk-space-with-onedrive-files-on-demand-for-windows-0e6860d3-d9f3-4971-b321-7092438fb38e

---

## 2. Architectural commitments CfAPI demands

### 2a. An always-running, registered provider whose callbacks must be available

The provider opens a **bi-directional channel** and hands the platform a **callback table**; the
platform calls back into the *running provider process* whenever it needs data or placeholders:

> "Initiates bi-directional communication between a sync provider and the sync filter API."
> "This parameter is how the sync provider tells the library which functions to call for various
> types of requests from the platform."
> "A sync provider typically calls this API soon after startup, once it has been initialized and is
> ready to service requests."
> "The sync root must be registered with the platform prior to being connected."
> — CfConnectSyncRoot
> https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfconnectsyncroot

The decisive limitation: when the provider stops servicing callbacks, **the platform fails the
operation**. There is no fallback — no provider, no file:

> "After a call to **CfDisconnectSyncRoot** returns, the sync provider will no longer receive
> callbacks and the platform will fail any operation that depends on said callbacks."
> — CfDisconnectSyncRoot
> https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfdisconnectsyncroot

Callbacks arrive on arbitrary pool threads, concurrently, and under a hard 60-second deadline — the
provider must be live and responsive, not a process that wakes up on demand:

> "Callback routines will be invoked in an arbitrary thread (part of a thread pool). Multiple
> callbacks can occur simultaneously, in different threads, and it is the responsibility of the sync
> provider code to implement any necessary synchronization to make this work reliably."
> "Every callback request has a fixed 60 second timeout."
> — CF_CALLBACK_TYPE
> https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type

The user-facing consequence of "provider/network not available" is that online-only files simply
cannot be opened. Microsoft says so plainly:

> "A blue cloud icon next to a OneDrive file or folder indicates that the file is only available
> online." … "You can't open online-only files when your device isn't connected to the Internet."
> — Files On-Demand for Windows (Microsoft Support)
> https://support.microsoft.com/en-us/office/save-disk-space-with-onedrive-files-on-demand-for-windows-0e6860d3-d9f3-4971-b321-7092438fb38e

In CfAPI terms a placeholder is "only available if the sync service is available":

> "**Placeholder file**: An empty representation of the file and only available if the sync service
> is available."
> — Build a Cloud Sync Engine
> https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine

### 2b. Registration is machine state that outlives the app

Registering a sync root mutates persistent OS state (Explorer nav-pane node, registry keys, the
cldflt minifilter). The Microsoft sample warns that if the provider crashes, the registration is
left behind and actively breaks Explorer until manually cleaned up:

> "To stop the sample, set focus to the console output and press **Ctrl-C**. This will cleanup the
> sync root registration so that the provider is uninstalled. If the sample crashes, it's possible
> that the sync root will remain registered. This will cause File Explorer to relaunch every time
> you click on anything, and you would get prompted for the fake client and server locations. If
> this occurs, uninstall the **CloudMirrorPackage** sample application from your computer."
> — Build a Cloud Sync Engine (Cloud Mirror sample)
> https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine

The placeholder/hydration machinery rides on a kernel **minifilter driver** that proxies between
apps and the provider — system-level plumbing, NTFS-only:

> "At the core of the storage stack in the cloud files API is a file system minifilter driver called
> cldflt.sys. This driver acts as a proxy between the user's applications and your sync engine. … it
> is responsibility of cldflt.sys to work with the Shell to present files as if the cloud data were
> locally available."
> "Cldflt.sys currently only supports NTFS volumes because it depends on some features unique to
> NTFS."
> — same page

Sync engines are also required to use the Desktop Bridge / packaged-app model:

> "Sync engines using the cloud files APIs are designed to use the [Desktop Bridge] as an
> implementation requirement."
> — same page

### 2c. The full callback surface a real provider must implement

CfAPI is callback-driven: the provider must implement a family of platform callbacks, not just "give
me bytes." The platform owns the file lifecycle and notifies the provider about hydration,
dehydration, rename, and delete:

> "These are not APIs provided by the library, but rather callbacks that a sync provider must
> implement in order to service requests from the platform. As necessary, the platform will ask the
> library instance running inside the sync provider process to invoke the appropriate callback
> routine."
> — CF_CALLBACK_TYPE
> https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type

Selected callbacks (verbatim descriptions, same page):

- `CF_CALLBACK_TYPE_FETCH_DATA` — "ask the sync provider for a range of file data that is required in
  order to satisfy an I/O request, or an explicit hydration request, on a placeholder. Implementation
  of this callback is required if the sync provider specifies a hydration policy that is *not*
  **ALWAYS_FULL** at the sync root registration time."
- `CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS` — "ask the sync provider to provide information about the
  contents of a placeholder directory to satisfy a directory query operation or an attempt to open a
  file underneath the directory. Implementation … required only if the sync provider specifies a
  policy other than **CF_POPULATION_POLICY_ALWAYS_FULL** …"
- `CF_CALLBACK_TYPE_VALIDATE_DATA` — "ask the sync provider for acknowledgement that a given range of
  file data … is valid …"
- `CF_CALLBACK_TYPE_NOTIFY_DEHYDRATE` — "inform the sync provider that a placeholder … is about to be
  dehydrated. The user application that performs the dehydration is blocked. A response is expected
  …"
- `CF_CALLBACK_TYPE_NOTIFY_DELETE` — "inform the sync provider that a placeholder … is about to be
  deleted. The user application that performs the deletion is blocked. A response is expected …"
- `CF_CALLBACK_TYPE_NOTIFY_RENAME` — "inform the sync provider that a placeholder … is about to be
  renamed or moved. The user application that performs the rename/move is blocked. A response is
  expected …"
- plus optional CANCEL_FETCH_DATA / CANCEL_FETCH_PLACEHOLDERS / FILE_OPEN/CLOSE completion /
  *_COMPLETION notifications.

Microsoft's own description of "what a fully-developed provider must implement" makes clear this is
bidirectional sync (download for hydration, upload on local change, conflict/merge handling) — i.e. a
sync engine, not a one-file fetch:

> "When the sync root file is just a placeholder, the service is responsible for copying down the
> contents of the file for hydration." … "When the sync root file is a full file and the contents of
> the file in the cloud service change, the service is responsible of notifying the local sync client
> of the change and the local sync client must handle merges according to their own specifications."
> … "When the sync root file is a full file and the contents of the file in the sync root path (the
> local client) change, the local sync client must notify the cloud service and handle merges …"
> — Build a Cloud Sync Engine
> https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine

### 2d. The implementation bar is steep (designers' own sample + practitioner reports)

Even Microsoft's reference sample disclaims production-readiness:

> "It is not intended to be used as production code. It lacks robust error handling and it is written
> to be as easily understood as possible."
> — Build a Cloud Sync Engine (Cloud Mirror sample)
> https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine

Practitioners attempting a real provider report substantial friction on the Microsoft Q&A "Cloud File
API FAQ":

> "Based on the introduction from Windows official documentation … and the **Cloud Mirror Sample**,
> we attempted to develop a SyncEngine independently. However, we encountered numerous issues. We
> believe other developers may face similar challenges …"
> "What is the relationship between these two? In the sample, `winrt::StorageProviderSyncRootManager::Register`
> is used, while the Cloud Filter API provides `CfRegisterSyncRoot`. … Must both be used? Or which
> one is recommended?"
> "Does the Cloud File API provide **logs** to assist in issue diagnosis? How can we quickly identify
> which specific interface timed out?"
> "What is the **default timeout duration** … Can this timeout be adjusted? If the timeout is short
> and non-adjustable, and user operations are frequent, what approach is recommended …?"
> — Cloud File API FAQ (Microsoft Q&A)
> https://learn.microsoft.com/en-us/answers/questions/2288103/cloud-file-api-faq

---

## 3. Genuine advantages of the placeholder approach

These are real and worth acknowledging — they are exactly why OneDrive uses CfAPI:

- **Instant browse of huge trees with ~zero local storage.** Placeholders are tiny stubs that
  hydrate on access:
  > "Sync engines can create placeholder files that consume only 1 KB of storage for the filesystem
  > header, and that automatically hydrate into full files under normal use conditions. Placeholder
  > files present as typical files to apps and to end users in the Windows Shell."
  > — https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine
  > "Online-only files don't take up space on your computer." … "When you open an online-only file,
  > it downloads to your device and becomes a locally available file."
  > — https://support.microsoft.com/en-us/office/save-disk-space-with-onedrive-files-on-demand-for-windows-0e6860d3-d9f3-4971-b321-7092438fb38e

- **No full download required; transparent app compatibility.**
  > "Whether you use file system APIs, the Command Prompt, or a desktop or a UWP app to access a
  > placeholder file, the file will hydrate without additional code changes and that app can use the
  > file normally."
  > — Build a Cloud Sync Engine (same URL)

- **First-class File Explorer integration** — nav-pane root, context-menu verbs, thumbnails/metadata,
  share/search handlers:
  > "Registering a sync root with the cloud files API causes that sync root (with an icon and custom
  > name) to appear in File Explorer's navigation pane." … "automatically provides several verbs …
  > in File Explorer's context menu that let the user control the hydration state of their file."
  > — same URL

- **Standardized status overlays / hydration progress** (replacing legacy overlay shell extensions):
  > "The cloud files API provides standardized, automatic hydration state icons shown in File
  > Explorer and on the Windows desktop." … "Replaces legacy icon overlay Shell extensions."
  > — same URL

---

## 4. Why CfAPI is a POOR FIT for a transient "open & edit one remote file"

Smart Explorer wants to open one remote file (SFTP/FTP/WebDAV/Drive), let the user edit it, and save
it back — and it is **not** an always-running sync daemon. CfAPI's design assumptions clash with that
on every axis:

1. **It assumes a continuously-running, registered provider.** CfAPI is for "a service that syncs
   files" whose callbacks are serviced "soon after startup, once it has been initialized and is ready
   to service requests" (CfConnectSyncRoot). A file-explorer that the user opens, uses, and closes is
   the opposite of a daemon. When the provider isn't running, "the platform will fail any operation
   that depends on said callbacks" (CfDisconnectSyncRoot) — so any placeholder left on disk becomes a
   dead, un-openable stub the moment the app exits.

2. **Registration is durable machine state that outlives the action.** A sync root creates an Explorer
   nav-pane node, registry keys, and ties into the cldflt minifilter; a crash can leave it registered
   such that "File Explorer [will] relaunch every time you click on anything" until you uninstall the
   app. That is a wildly disproportionate footprint for "edit one file." (Cloud Mirror sample notes,
   same Build-a-Sync-Engine URL.)

3. **It demands the full bidirectional sync + callback surface**, not a fetch. A real provider must
   implement FETCH_DATA, FETCH_PLACEHOLDERS, and handle NOTIFY_DEHYDRATE/DELETE/RENAME, plus upload
   and conflict/merge on local change ("the local sync client must notify the cloud service and handle
   merges according to their own specifications"). For a one-shot open/edit/save, almost all of this
   is dead weight.

4. **Placeholders persist on disk and represent an ongoing namespace the app must keep coherent.** The
   placeholder/full/pinned state model assumes long-lived files the engine continuously reconciles —
   not a temporary working copy.

5. **The implementation/operational bar is high and platform-constrained** — desktop-only, NTFS-only,
   Desktop Bridge packaging required, 60-second hard callback timeouts, multi-threaded reentrant
   callbacks, and (per practitioners) thin logging/diagnostics. Microsoft's own sample "is not
   intended to be used as production code." This is the cost of building OneDrive, not of opening a
   file.

**Where Microsoft/practitioners effectively say a full placeholder engine is overkill:** CfAPI's
documented users are "typically sync providers" (the portal page) — i.e. the tool is scoped to
sync-engine builders, not transient editor apps. The whole value proposition (1 KB stubs, instant
browse of an entire cloud namespace, Explorer overlays, dehydration to reclaim space) only pays off
when you are presenting and continuously reconciling a *large, persistent* remote tree. For a single
"download a real copy, edit, save back" action by a non-daemon app, none of those benefits apply and
all of the architectural commitments (always-running registered provider, durable machine-state
registration, full callback/sync surface, kernel minifilter dependency) are pure cost. Approach (B) —
download a real local copy and write it back — matches the actual workload; Approach (A) is the OneDrive
architecture and is over-engineered for it.

---

## Sources
- Build a Cloud Sync Engine that Supports Placeholder Files — https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine
- Cloud Sync Engines (Cloud Files API portal) — https://learn.microsoft.com/en-us/windows/win32/cfapi/cloud-files-api-portal
- CfConnectSyncRoot — https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfconnectsyncroot
- CfDisconnectSyncRoot — https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfdisconnectsyncroot
- CF_CALLBACK_TYPE — https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type
- Save disk space with OneDrive Files On-Demand for Windows (Microsoft Support) — https://support.microsoft.com/en-us/office/save-disk-space-with-onedrive-files-on-demand-for-windows-0e6860d3-d9f3-4971-b321-7092438fb38e
- Cloud File API FAQ (Microsoft Q&A, practitioner report) — https://learn.microsoft.com/en-us/answers/questions/2288103/cloud-file-api-faq
