# Lazy virtual-files IDataObject for remote clipboard/drag-out — recorded API syntax

Checked 2026-09-28 against Microsoft Learn, The Old New Thing (devblogs.microsoft.com),
and the locally vendored crate sources pinned by `native/Cargo.toml`'s direct
dependencies — `windows` 0.58.0, `windows-core` 0.58.0, `windows-implement`
0.58.0 (under `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`)
and `windows-sys` 0.59.0 (Cargo.lock's other `windows*`/`windows-sys` versions
are transitive, for `eframe`/`rfd`/`keyring`, and irrelevant here). Existing
local code read for style only (not edited):
`native/src/virtual_clipboard/os/{windows,read_stream}.rs`,
`native/src/dragout/os/windows.rs`.

## 1. CFSTR_FILEDESCRIPTORW / FILEGROUPDESCRIPTORW / FILEDESCRIPTORW

Source: [FILEDESCRIPTORW](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/ns-shlobj_core-filedescriptorw),
[Shell Clipboard Formats](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard),
crate `windows-0.58.0/.../UI/Shell/mod.rs:34633` (`FILEDESCRIPTORW`), `:34670`
(`FILEGROUPDESCRIPTORW`).

```rust
pub struct FILEDESCRIPTORW {
    pub dwFlags: u32, pub clsid: windows_core::GUID,       // 4 + 16
    pub sizel: Foundation::SIZE, pub pointl: Foundation::POINTL, // 8 + 8 (icon geometry)
    pub dwFileAttributes: u32,                             // 4
    pub ftCreationTime: FILETIME, pub ftLastAccessTime: FILETIME, pub ftLastWriteTime: FILETIME, // 8*3
    pub nFileSizeHigh: u32, pub nFileSizeLow: u32,          // 4 + 4
    pub cFileName: [u16; 260],                             // 520 — MAX_PATH, UTF-16, NUL-terminated
}
pub struct FILEGROUPDESCRIPTORW { pub cItems: u32, pub fgd: [FILEDESCRIPTORW; 1] }
```
`size_of::<FILEDESCRIPTORW>() == 592` bytes (sum above — cross-confirmed by a
third-party doc excerpt). `fgd: [FILEDESCRIPTORW; 1]` is a flexible-array-
member; real size is `size_of::<FILEGROUPDESCRIPTORW>() +
(cItems-1)*size_of::<FILEDESCRIPTORW>()`, exactly what
`virtual_clipboard/os/windows.rs::make_descriptor` (74–111) already computes
and writes via `ptr::write_unaligned` (the struct is bindgen'd unaligned).

**cFileName / relative paths**: MS documents only "the name of the file", not
relative paths. This repo's **shipped** `virtual_clipboard/os/windows.rs`
(header comment; `rel.replace('/', "\\")` at line 89) already relies on
`cFileName` holding a backslash relative path ("Projekt\Unterordner\datei.txt")
with Explorer recreating intermediate directories on paste — the same
mechanism zipped folders/Outlook attachments use, proven working in
production by this codebase. No separate directory descriptor is emitted for
those intermediates — Explorer derives them from file-entry path components
alone. Limit is 260 UTF-16 units (259 + NUL); existing code guards this
(`rel.encode_utf16().count() >= 260` → `E_INVALIDARG`, line 264).

**FD_* flags** (`.../UI/Shell/mod.rs:25550-25559`, matches MS exactly):

| Flag | Value | Meaning |
|---|---|---|
| `FD_CLSID` | 0x1 | `clsid` valid |
| `FD_SIZEPOINT` | 0x2 | `sizel`/`pointl` valid |
| `FD_ATTRIBUTES` | 0x4 | `dwFileAttributes` valid (needed for `FILE_ATTRIBUTE_DIRECTORY`) |
| `FD_CREATETIME` | 0x8 | `ftCreationTime` valid |
| `FD_ACCESSTIME` | 0x10 | `ftLastAccessTime` valid |
| `FD_WRITESTIME` | 0x20 | `ftLastWriteTime` valid (already used) |
| `FD_FILESIZE` | 0x40 | size valid; also required (with 0/0) to create a zero-length file |
| `FD_PROGRESSUI` | 0x4000 | show progress for this entry (already used) |
| `FD_LINKUI` | 0x8000 | legacy "this is a shortcut"; superseded by `CFSTR_PREFERREDDROPEFFECT=DROPEFFECT_LINK` |
| `FD_UNICODE` | 0x80000000 | Vista+: descriptor is Unicode (moot — always true for the W format) |

**Directories**: set `dwFileAttributes = FILE_ATTRIBUTE_DIRECTORY` +
`FD_ATTRIBUTES`. A narkive/Usenet thread
(["CFSTR_FILECONTENTS and CFSTR_FILEDESCRIPTOR clipboard formats"](https://microsoft.public.platformsdk.shell.narkive.com/irFLJjTw/cfstr-filecontents-and-cfstr-filedescriptor-clipboard-formats))
confirms Explorer creates a real (empty) directory for such an entry — so
**empty directories work**. **Ordering** (parents before children?): not
documented; strong indirect evidence it does *not* matter — this repo's
shipped code emits a flat, unsorted file list and nested paths already work
in production, implying Explorer resolves the union of path prefixes as a
set. Exact ordering with an *explicit* empty-directory entry is unverified
(§9). **Large counts**: no documented ceiling; 100k entries ≈ 56.5 MiB in one
`GMEM_MOVEABLE` block — fine on 64-bit Windows but a known perf/memory
concern for big drags, which is exactly what `FD_PROGRESSUI` +
`IDataObjectAsyncCapability` (§4) mitigate. **Duplicate/invalid names**:
undocumented, presumed to follow the Shell copy engine's normal
already-exists/illegal-character handling, but **not verified** for this
path specifically (§9).

## 2. CFSTR_FILECONTENTS / TYMED_ISTREAM / lindex

Source: [Shell Clipboard Formats](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard),
[Handling Shell Data Transfer Scenarios](https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios),
[Old New Thing: IStream edition](https://devblogs.microsoft.com/oldnewthing/20080319-00/?p=23073).

- `lindex` is the **zero-based index** into the paired `CFSTR_FILEDESCRIPTOR`
  array — Old New Thing flags this as a common trap ("`lindex` is zero, not
  `-1`"), unlike most other clipboard formats. Matches existing `GetData`
  (`windows.rs:127-141`): `fe.lindex` used directly, `DV_E_LINDEX` if OOB.
  One `GetData(CFSTR_FILECONTENTS, lindex=i)` call per file, not one blob.
- **Which IStream methods Explorer actually calls**: not exhaustively
  documented; confirmed by this repo's **shipped, working** `read_stream.rs`:
  `Read` (required; short read ⇒ `S_FALSE`=EOF, full read ⇒ `S_OK`, per the
  general `ISequentialStream` contract) and `Stat`, which branches on
  `STATFLAG_NONAME` (skips `pwcsName`/`CoTaskMemAlloc` when set) — proof
  Explorer calls `Stat` with `STATFLAG_NONAME` at least sometimes. `cbSize`
  is filled from real metadata; not documented as strictly mandatory (the
  paired `FILEDESCRIPTORW` size already carries it) but treated as a
  required cross-check. `Seek`/`CopyTo`/`Clone` are implemented defensively
  (`Arc<Mutex<File>>` + per-clone `Mutex<u64>` cursor) but **not confirmed
  required** — the Old New Thing sample's `Seek(0, STREAM_SEEK_END, …)` is
  the *source* sizing its own helper stream, not evidence of what Explorer
  calls on a vended one. `SetSize`/`Revert`/`Lock`/`UnlockRegion`/`Write`
  correctly refuse (`STG_E_ACCESSDENIED`/`STG_E_INVALIDFUNCTION`), as today.
- **Forward-only stream acceptable?** Undocumented either way. Since `Seek`
  must exist on the vtable regardless, implement at least forward `Seek`
  (`CUR`/`SET` ≥ current, `END` for size discovery) and buffer-or-fail
  backward seeks; whether Explorer ever issues one is **unresolved** (§9).
  Network backends that can only stream forward may need a read-ahead buffer.
- **Directories must not get CFSTR_FILECONTENTS requests**: not explicit in
  any source, but implied by the format's design (no "contents" for a
  directory), consistent with `CF_HDROP`. Defensive fallback: answer an
  unexpected directory-index content request with a zero-length stream
  rather than an error.

## 3. Optional communication formats

Source: [Shell Clipboard Formats § Communication](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard),
[Handling Shell Data Transfer Scenarios](https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios).

- `CFSTR_PREFERREDDROPEFFECT` (`"Preferred DropEffect"`, `TYMED_HGLOBAL` →
  `DWORD DROPEFFECT_*`): source→target hint; already implemented
  (`windows.rs:143-155`, hard-codes `DROPEFFECT_COPY`).
- `CFSTR_PERFORMEDDROPEFFECT`/`CFSTR_PASTESUCCEEDED` (target→source via
  `SetData`): negotiate optimized-move/delete-on-paste. Both existing
  `SetData` implementations (virtual_clipboard, dragout) unconditionally
  return `E_NOTIMPL`/`DV_E_FORMATETC` — fine for copy-only sources today;
  offering `DROPEFFECT_MOVE` later would require accepting these (store,
  no-op) or Explorer's move bookkeeping breaks silently.
- `CFSTR_TARGETCLSID` (`"TargetCLSID"`, `TYMED_HGLOBAL` → CLSID): set via
  `SetData` when dropped on the **Recycle Bin** (`CLSID_RecycleBin`) —
  relevant only if remote drag-to-delete is ever supported.
- All are plain `RegisterClipboardFormatW` formats, `TYMED_HGLOBAL`,
  `DWORD`-sized — same shape as the existing `cf_dropeffect` handling.

## 4. IDataObjectAsyncCapability (a.k.a. IAsyncOperation)

Source: [interface](https://learn.microsoft.com/en-us/windows/win32/api/shldisp/nn-shldisp-idataobjectasynccapability),
[::StartOperation](https://learn.microsoft.com/en-us/windows/desktop/api/shldisp/nf-shldisp-idataobjectasynccapability-startoperation),
[::EndOperation](https://learn.microsoft.com/en-us/windows/desktop/api/shldisp/nf-shldisp-idataobjectasynccapability-endoperation),
[datascenarios § async](https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios),
crate `.../UI/Shell/mod.rs:9360-9399` + `.../UI/Shell/impl.rs:5456-5530`.

GUID `3d8b0590-f691-11d2-8ea9-006097df5bd4`, `shldisp.h`, `Shell32.dll` v6+,
min. client **Windows 8**. Renamed from `IAsyncOperation` (identical shape).

Call sequence (drag-and-drop, per *datascenarios*): (1) source calls
`SetAsyncMode(TRUE)` before `DoDragDrop`; (2) on `IDropTarget::Drop`,
**target** QIs for the interface and calls `GetAsyncMode`; (3) if true,
target spawns **its own background thread** (source doesn't choose or know
which) and calls `StartOperation(nullptr)`; (4) target returns from `Drop`
immediately, unblocking `DoDragDrop`, and extracts on that background
thread; (5) target calls `SetData` (performed-effect/paste-succeeded) if
applicable, then `EndOperation(hresult, nullptr, dwEffects)`; (6) source
calls `InOperation` after `DoDragDrop` returns: false/failure ⇒ already
finished synchronously; true ⇒ still running, and per MS text the object is
notified of completion "through a **private** interface" — not a generically
hookable callback, so a source just stays alive and keeps answering
`GetData`/`Read` until COM releases it. Source-side obligation is only to
implement all 5 methods (store/report a bool + in-operation state); the
**target** creates the background thread, never the source.
- **Applies to clipboard paste too?** Yes per the interface's own remarks:
  "primarily exported by data objects used with drag-and-drop **and
  Clipboard operations**" — but *datascenarios* only spells out the numbered
  sequence in `DoDragDrop`/`IDropTarget::Drop` terms; paste is presumed
  analogous (Explorer plays "target" after `OleGetClipboard`) by inference,
  **not separately confirmed** (§9).

Exact `_Impl` trait (verbatim, `.../UI/Shell/impl.rs:5456-5462`):
```rust
pub trait IDataObjectAsyncCapability_Impl: Sized {
    fn SetAsyncMode(&self, fdoopasync: Foundation::BOOL) -> windows_core::Result<()>;
    fn GetAsyncMode(&self) -> windows_core::Result<Foundation::BOOL>;
    fn StartOperation(&self, pbcreserved: Option<&Com::IBindCtx>) -> windows_core::Result<()>;
    fn InOperation(&self) -> windows_core::Result<Foundation::BOOL>;
    fn EndOperation(&self, hresult: windows_core::HRESULT, pbcreserved: Option<&Com::IBindCtx>, dweffects: u32) -> windows_core::Result<()>;
}
```
Gating: module needs `Win32_UI_Shell` (enabled); `StartOperation`/
`EndOperation` additionally need `#[cfg(feature = "Win32_System_Com")]`
(enabled). **No new Cargo feature required.**

## 5. COM threading: OleSetClipboard, apartments, and agility

Source: [OleSetClipboard](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-olesetclipboard),
[OleIsCurrentClipboard](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-oleiscurrentclipboard),
[OleFlushClipboard](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-oleflushclipboard),
[IAgileObject](https://learn.microsoft.com/en-us/windows/win32/api/objidlbase/nn-objidlbase-iagileobject),
crate `windows-implement-0.58.0/src/lib.rs:236-254`,
`windows-core-0.58.0/src/imp/weak_ref_count.rs:190-210`.

- **STA + pumped loop required.** `OleSetClipboard` "assigns ownership of the clipboard to an
  **internal OLE window handle**" on the calling thread, whose WndProc "satisfies `WM_RENDERFORMAT`...
  by delegating to the `IDataObject`" — matches `dragout/os/windows.rs`'s existing comment ("OLE
  already initialised on this (main) thread"). Call `OleSetClipboard` **directly on the dedicated
  worker STA thread** that owns the object — no marshaling needed for clipboard (only `DoDragDrop`
  on the GUI thread needs the marshaled proxy, §6). Delayed rendering: "the clipboard contains only
  a pointer to the data object" until `OleFlushClipboard`/`OleSetClipboard(NULL)` releases it (also
  dropping the one AddRef `OleSetClipboard` took).
- **Another thread later calling `OleSetClipboard`/`SetClipboardData`**: per docs, the *previous*
  object's `IDataObject` pointer "is released... the clipboard owner should use this as a signal."
  Because our object unconditionally answers `QueryInterface(IID_IAgileObject)` (below), **that
  `Release` is not guaranteed to land on our worker thread** — rely on refcount/`Drop`, or poll,
  not "only my own pump sees this".
- **`OleIsCurrentClipboard`** only answers "is *this exact* object still current", explicitly *not*
  usable to ask "what's on the clipboard now" ("cannot be called by the consumer"). Use it, or the
  already-used `GetClipboardSequenceNumber`, from a `WM_TIMER` on the worker thread to detect
  ownership loss — there is no push notification to an arbitrary pumping thread; a real push needs
  our own hidden window + `AddClipboardFormatListener`/`WM_CLIPBOARDUPDATE`.
- **`OleFlushClipboard` must be avoided.** It "renders the data... onto the clipboard", and for an
  `IStorage`/`IStream`-backed format "the storage object is copied into memory" — i.e. it forces
  **every** offered format, including **every file's `CFSTR_FILECONTENTS`**, to render immediately,
  defeating the lazy design for a 100k-entry tree outright. Only call it to survive process exit
  (not applicable — remote content needs this process alive to stream). Different mechanism from
  raw `SetClipboardData`/`WM_RENDERFORMAT`, which has a documented **30 s** render timeout
  ([Old New Thing](https://devblogs.microsoft.com/oldnewthing/20220609-00/?p=106731)) — no equivalent
  fixed timeout was found for the OLE `IDataObject::GetData` path itself (§8).
- **Worker-thread pattern**: `OleInitialize(None)` → build object → `OleSetClipboard(&obj)` →
  `GetMessageW`/`TranslateMessage`/`DispatchMessageW` loop with `SetTimer(HWND::default(), …)`
  (thread timer, no window needed) polling ownership; on loss, quit the loop, `OleUninitialize`, exit.
- **Do Explorer's calls dispatch onto our worker thread, or can they land elsewhere?** Key finding:
  **`#[implement]` objects in windows-rs 0.58 unconditionally answer `QueryInterface(IID_IAgileObject)`
  with success** — verbatim, `windows-implement-0.58.0/src/lib.rs:239-243`:
  ```rust
  let interface_ptr: *mut ::core::ffi::c_void = if iid == &<::windows_core::IUnknown as ::windows_core::Interface>::IID
      || iid == &<::windows_core::IInspectable as ::windows_core::Interface>::IID
      || iid == &<::windows_core::imp::IAgileObject as ::windows_core::Interface>::IID {
          &self.identity as *const _ as *mut _
  }
  ```
  No opt-out exists in 0.58 — every `#[implement(...)]` type is agile. Per
  [IAgileObject](https://learn.microsoft.com/en-us/windows/win32/api/objidlbase/nn-objidlbase-iagileobject):
  "the object is called directly in the new apartment, rather than marshaling." Consequence:
  **incoming calls are not guaranteed to execute on the thread that created the object** —
  cross-process calls land on whatever RPC/LRPC worker thread services them; in-process handoffs
  (§6) skip proxy/stub thread-switching entirely. `GetData`/`Read`/`Stat` must be internally
  thread-safe on their own merits (already true via the `Mutex`-guarded file/cursor in
  `read_stream.rs`) — never assume "network I/O only happens on the thread that created it";
  serialize any non-thread-safe session/handle explicitly. (Weaker/secondary corroboration:
  `windows-core-0.58.0/src/imp/weak_ref_count.rs:206-210` matches `IAgileObject::IID` too, but only
  for the internal `IWeakReference` tear-off identity, not the object itself.)
- `windows_core::AgileReference<T>` is **not usable in this pinned version**: `agile_reference.rs`
  exists on disk in `windows-core-0.58.0/src/` but its module is **not declared** in that crate's
  `lib.rs` (confirmed by reading the full 60-line file — no `mod agile_reference;`), so it can't be
  imported. Raw `RoGetAgileReference` exists only inside `#[doc(hidden)] pub mod imp` — usable but
  explicitly undocumented/unsupported; prefer §6 instead.

## 6. Marshaling worker-thread object to GUI thread for DoDragDrop

Source: [CoMarshalInterThreadInterfaceInStream](https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-comarshalinterthreadinterfaceinstream),
[CoGetInterfaceAndReleaseStream](https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-cogetinterfaceandreleasestream),
[Old New Thing: the rules](https://devblogs.microsoft.com/oldnewthing/20151021-00/?p=91311),
[DoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop),
crate `.../System/Com/Marshal/mod.rs:116-123`, `.../StructuredStorage/mod.rs:25-33`.

```rust
// Marshal/mod.rs:116 — feature "Win32_System_Com_Marshal" (NOT enabled today — GAP, §7)
pub unsafe fn CoMarshalInterThreadInterfaceInStream<P0>(riid: *const windows_core::GUID, punk: P0) -> windows_core::Result<super::IStream>
where P0: windows_core::Param<windows_core::IUnknown>;

// StructuredStorage/mod.rs:25 — feature "Win32_System_Com_StructuredStorage" (already enabled)
pub unsafe fn CoGetInterfaceAndReleaseStream<P0, T>(pstm: P0) -> windows_core::Result<T>
where P0: windows_core::Param<super::IStream>, T: windows_core::Interface;
```

- Rules (Old New Thing): **originating apartment** (worker thread) calls `CoMarshalInterThreadInterfaceInStream`;
  the returned `IStream` "is safe for multi-threaded use" — hand it to the GUI thread via any channel.
  **Receiving apartment** (GUI thread) calls `CoGetInterfaceAndReleaseStream`, which always releases the
  stream — "even if the unmarshaling fails" — never touch it again either way. Apartment-threaded objects get
  a thread-bound proxy on unmarshal; free-threaded ones "just give you a direct pointer" — exactly our case
  per §5's `IAgileObject` finding, so this documented path is cheap here and the forward-compatible choice
  over depending on the undocumented agile-by-default behavior directly. Abort before handoff by calling
  `CoReleaseMarshalData` from the **originating** apartment; if that thread exits while a GUI-thread proxy
  still references it, the proxy fails with `RPC_E_SERVER_DIED_DNE`.
- `DoDragDrop` (unchanged usage vs. `dragout/os/windows.rs`): "must call `OleInitialize` before calling this
  function"; pumps its own loop while driving `IDropSource`/`IDropTarget`; returns `DRAGDROP_S_DROP`/
  `DRAGDROP_S_CANCEL`/`E_UNSPEC`. Runs on the GUI thread with the unmarshaled proxy; calls the drop target
  makes afterward land wherever COM dispatches them (§5) — not necessarily the worker thread, so internal
  thread-safety is what matters, not apartment ownership.
- Alternative for a longer-lived (non one-shot) registration:
  `IGlobalInterfaceTable` (`Com/mod.rs:2526-2540`, methods
  `RegisterInterfaceInGlobal`/`RevokeInterfaceFromGlobal`/`GetInterfaceFromGlobal`,
  feature `Win32_System_Com`, already enabled) — not obviously needed here.

## 7. Exact windows 0.58 signatures, file paths, feature gaps

Enabled today (`windows` dependency in `native/Cargo.toml`): `implement`,
`Win32_Foundation`, `Win32_Graphics_Gdi`, `Win32_Security`,
`Win32_Storage_FileSystem`, `Win32_System_Com`, `Win32_System_Power`,
`Win32_System_Com_StructuredStorage`, `Win32_System_DataExchange`,
`Win32_System_Memory`, `Win32_System_Ole`, `Win32_System_SystemServices`,
`Win32_UI_Input_KeyboardAndMouse`, `Win32_UI_Shell`, `Win32_UI_Shell_Common`,
`Win32_UI_WindowsAndMessaging`, `Networking_Connectivity`. Paths below are
relative to `.../index.crates.io-1949cf8c6b5b557f/`.

| API | File:line | Feature needed | Status |
|---|---|---|---|
| `OleInitialize`/`OleUninitialize` | `windows-0.58.0/.../System/Ole/mod.rs:557,971` | `Win32_System_Ole` | OK |
| `OleSetClipboard`/`OleIsCurrentClipboard` | `.../Ole/mod.rs:766,563` | `Win32_System_Ole` + `Win32_System_Com` | OK |
| `OleFlushClipboard` | `.../Ole/mod.rs:505` | `Win32_System_Ole` | OK |
| `DoDragDrop` | `.../Ole/mod.rs:95` | `Win32_System_Ole` | OK (used) |
| `IDropSource_Impl` | `.../Ole/impl.rs:992` | `Win32_System_Ole` + `Win32_System_SystemServices` | OK (used) |
| `CoMarshalInterThreadInterfaceInStream` | `.../System/Com/Marshal/mod.rs:116` | **`Win32_System_Com_Marshal`** | **GAP — add** |
| `CoGetInterfaceAndReleaseStream` | `.../System/Com/StructuredStorage/mod.rs:25` | `Win32_System_Com_StructuredStorage` | OK |
| `IGlobalInterfaceTable` | `.../System/Com/mod.rs:2526` | `Win32_System_Com` | OK |
| `IDataObject_Impl` | `.../System/Com/impl.rs:1885` | `Win32_Graphics_Gdi` **and** `Win32_System_Com_StructuredStorage` | OK (used) |
| `ISequentialStream_Impl`/`IStream_Impl` | `.../System/Com/impl.rs:4621,4753` | `Win32_System_Com` | OK (used) |
| `IDataObjectAsyncCapability`(+`_Impl`) | `.../UI/Shell/mod.rs:9360` / `impl.rs:5456` | `Win32_UI_Shell` + `Win32_System_Com` | OK |
| `SHCreateStdEnumFmtEtc` | `.../UI/Shell/mod.rs:2929` | `Win32_UI_Shell` | OK (used) |
| `FILEDESCRIPTORW`/`FILEGROUPDESCRIPTORW`/`FD_*`/`DROPFILES` | `.../UI/Shell/mod.rs:34633,34670,25550,34506` | `Win32_UI_Shell` | OK (used) |
| `GetMessageW`/`PeekMessageW`/`TranslateMessage`/`DispatchMessageW` | `.../UI/WindowsAndMessaging/mod.rs:1360,2408,3230,770` | `Win32_UI_WindowsAndMessaging` | OK |
| `PostThreadMessageW` | `.../WindowsAndMessaging/mod.rs:2458` | `Win32_UI_WindowsAndMessaging` | OK |
| `MsgWaitForMultipleObjects`/`…Ex` | `.../WindowsAndMessaging/mod.rs:2347,2355` | `Win32_UI_WindowsAndMessaging` | OK |
| `SetTimer`/`KillTimer` | `.../WindowsAndMessaging/mod.rs:2992,1879` | `Win32_UI_WindowsAndMessaging` | OK |
| `GlobalAlloc`/`GlobalLock`/`GlobalUnlock` | `.../System/Memory/mod.rs:162,182,207` | `Win32_System_Memory` | OK (used) |
| `GlobalFree` | `.../Foundation/mod.rs:51` (**not** in `System/Memory`) | `Win32_Foundation` | OK |
| `GetCurrentThreadId` | `.../System/Threading/mod.rs:828` | **`Win32_System_Threading`** on `windows` | **GAP** — only `windows-sys` 0.59 has it today; add the feature, or call `windows_sys::Win32::System::Threading::GetCurrentThreadId` instead |

Two concrete `native/Cargo.toml` gaps in the `windows` feature list: add
**`Win32_System_Com_Marshal`** (for `CoMarshalInterThreadInterfaceInStream`);
add **`Win32_System_Threading`** only if `GetCurrentThreadId` should come
from `windows` rather than the already-enabled `windows-sys` one. Everything
else needed is already enabled.

## 8. Pitfalls

Source: [IDataObject::GetData](https://learn.microsoft.com/en-us/windows/win32/api/objidl/nf-objidl-idataobject-getdata),
[Old New Thing: >30s delay-rendered clipboard wait](https://devblogs.microsoft.com/oldnewthing/20220609-00/?p=106731).

- **`GetData` HRESULTs** (exact MS table): `DV_E_LINDEX` (index outside
  `[0, cItems)` — the "-1 only" boilerplate wording doesn't apply to
  `CFSTR_FILECONTENTS`, any valid 0-based index is fine), `DV_E_FORMATETC`,
  `DV_E_TYMED`, `DV_E_DVASPECT`, `OLE_E_NOTRUNNING`, `STG_E_MEDIUMFULL`
  (map `GlobalAlloc` failure here), `E_UNEXPECTED`, `E_INVALIDARG`,
  `E_OUTOFMEMORY`. **`E_PENDING` is not part of the documented `GetData`
  contract** — async deferral goes entirely through
  `IDataObjectAsyncCapability` (§4), not a "come back later" return from
  `GetData` itself; once called, it's expected to block until it can answer.
- **`Read` failing mid-transfer**: no documented specific Explorer UI
  behavior found. Since Explorer's copy engine treats our `IStream` like a
  real file handle, a generic copy-error dialog (Retry/Skip/Cancel) is the
  reasonable expectation — an **inference**, not a cited fact. Map the
  network error to a real `HRESULT` (as `read_stream.rs::com_error()`
  already does: `raw_os_error()` → `HRESULT::from_win32`, else
  `STG_E_READFAULT`) so the failure is at least legible.
- **`S_FALSE` at EOF**: confirmed both by the general `ISequentialStream`
  contract and by `read_stream.rs::Read` itself (`bytes == count` → `S_OK`,
  else `S_FALSE`) — a short read *is* the documented EOF signal, no separate
  EOF HRESULT exists.
- **Maximum practical sizes**: no documented ceiling (see §1's ~56.5 MiB/100k
  math); the whole `FILEGROUPDESCRIPTORW` blob is marshaled as one unit on
  every descriptor `GetData`, so budget real memory/time at that scale.
- **Timeouts**: the documented **30 s** `WM_RENDERFORMAT` render timeout
  applies only to the **raw Win32** delayed-rendering path
  (`SetClipboardData`/`WM_RENDERFORMAT`), not to the OLE
  `IDataObject::GetData`/`IStream::Read` path used here — no equivalent fixed
  timeout found documented for it, but don't assume cross-process COM/RPC
  calls are unbounded either; bound network stalls on our own side
  regardless (timeout/retry, surface an error).
- `pUnkForRelease == NULL` in the returned `STGMEDIUM` ⇒ **receiver owns and
  frees the medium** (`GlobalFree` for `TYMED_HGLOBAL`, `Release()` for
  `TYMED_ISTREAM`) — matches existing code (never frees its own
  descriptor/dropeffect `HGLOBAL`s, never double-releases the `IStream`).

## 9. Unresolved / not empirically verified

- Entry-order requirements when an *explicit* empty-directory entry and its
  descendants are mixed arbitrarily (only inferred from this repo's
  order-independent, file-only shipped behavior).
- Whether Explorer ever issues a **backward** `Seek` on a vended
  `CFSTR_FILECONTENTS` stream, versus purely forward `Read`.
- Exact duplicate-filename/invalid-character handling for
  `CFSTR_FILEDESCRIPTORW` paste specifically (vs. plain `CF_HDROP`).
- Whether the `IDataObjectAsyncCapability` paste sequence really mirrors the
  documented drag-and-drop one step-for-step (only general remarks, not a
  numbered walkthrough, confirm paste applicability).
- Any undocumented cross-process COM/RPC call timeout for a slow
  `GetData`/`Read`, and the practical wall-clock/memory ceiling for 100k+
  descriptor entries in one call — no measured numbers found for either.

## Sources

- [Handling Shell Data Transfer Scenarios](https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios) · [Shell Clipboard Formats](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard) · [FILEDESCRIPTORW](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/ns-shlobj_core-filedescriptorw)
- [IDataObjectAsyncCapability](https://learn.microsoft.com/en-us/windows/win32/api/shldisp/nn-shldisp-idataobjectasynccapability) · [::StartOperation](https://learn.microsoft.com/en-us/windows/desktop/api/shldisp/nf-shldisp-idataobjectasynccapability-startoperation) · [::EndOperation](https://learn.microsoft.com/en-us/windows/desktop/api/shldisp/nf-shldisp-idataobjectasynccapability-endoperation)
- [IDataObject::GetData](https://learn.microsoft.com/en-us/windows/win32/api/objidl/nf-objidl-idataobject-getdata)
- [OleSetClipboard](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-olesetclipboard) · [OleIsCurrentClipboard](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-oleiscurrentclipboard) · [OleFlushClipboard](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-oleflushclipboard)
- [DoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop)
- [CoMarshalInterThreadInterfaceInStream](https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-comarshalinterthreadinterfaceinstream) · [CoGetInterfaceAndReleaseStream](https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-cogetinterfaceandreleasestream)
- [IAgileObject](https://learn.microsoft.com/en-us/windows/win32/api/objidlbase/nn-objidlbase-iagileobject)
- The Old New Thing: [virtual file, HGLOBAL edition](https://devblogs.microsoft.com/oldnewthing/20080318-00/?p=23083) · [IStream edition](https://devblogs.microsoft.com/oldnewthing/20080319-00/?p=23073) · [multiple virtual objects](https://devblogs.microsoft.com/oldnewthing/20080331-00/?p=22933) · [CoMarshalInterThreadInterfaceInStream rules](https://devblogs.microsoft.com/oldnewthing/20151021-00/?p=91311) · [>30s delay-rendered clipboard wait](https://devblogs.microsoft.com/oldnewthing/20220609-00/?p=106731)
- narkive/Usenet: [CFSTR_FILECONTENTS and CFSTR_FILEDESCRIPTOR clipboard formats](https://microsoft.public.platformsdk.shell.narkive.com/irFLJjTw/cfstr-filecontents-and-cfstr-filedescriptor-clipboard-formats) (empty-directory creation)
- Local crate sources: `windows-0.58.0`, `windows-core-0.58.0`, `windows-implement-0.58.0` under `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/` (exact files/lines cited inline)
- This repo: `native/src/virtual_clipboard/os/windows.rs`, `.../virtual_clipboard/os/read_stream.rs`, `native/src/dragout/os/windows.rs`, `native/Cargo.toml`, `native/Cargo.lock`
