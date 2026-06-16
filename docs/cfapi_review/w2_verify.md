# W2 — Independent fact-check of the W1 CfAPI audit (claims A–G)

Independent verification of the highest-severity claims from the W1 CfAPI audit of
Smart Explorer, going to PRIMARY sources only: the `cloud-filter` v0.0.6 crate
source and Microsoft Learn `cfapi.h` / `Windows.Storage.Provider` docs. Each claim
below is re-derived from source — the W1 documents were read but treated as
unproven.

- Crate root: `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cloud-filter-0.0.6/src/`
- Our code: `/home/user/smart-explorer/native/src/cfprovider.rs`
- Verification date: 2026-06-16. Crate version: 0.0.6.

Legend: **CONFIRMED** = claim fully proven by primary source; **REFUTED** = claim
is wrong; **PARTIAL** = core is right but an element is overstated/imprecise;
**UNCERTAIN** = primary sources do not settle it.

---

## A. TRANSFER_DATA requires Offset AND Length 4096-aligned, except the final chunk ending at logical file size. Crate `write_at` passes raw (no alignment/chunking).

**Verdict: CONFIRMED.**

**MS authoritative source** — CF_OPERATION_PARAMETERS, `TransferData`
(<https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_operation_parameters>),
quoted verbatim:

> "*OpParams.TransferData.Offset* and *OpParams.TransferData.Length* describe a
> range in the placeholder to which the sync provider is transferring the data.
> There is no requirement that the sync provider return all data as requested in
> one shot. It is also OK for a sync provider to return more data than requested.
> ... The sync provider can also perform multiple **TRANSFER_DATA** operations
> repeatedly as a response to the same **FETCH_DATA** callback. The only
> requirement is that both offset and length are 4KB aligned unless the range
> described ends on the logical file size (EoF), in which case, the length is not
> required to be 4KB aligned as long as the resulting range ends on or beyond the
> logical file size."

Per-field, same page:

> `TransferData.Offset` — "... *Offset* must be 4KB aligned."
>
> `TransferData.Length` — "The *Length* in bytes of the *Buffer*. The length must
> be 4KB aligned unless the range described ends on the logical file size (EoF),
> in which case, the *Length* is not required to be 4KB aligned as long as the
> resulting range ends on or beyond the logical file size."

Restated from the requesting side — CF_CALLBACK_PARAMETERS, `FetchData`
(<https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_callback_parameters>):

> "However, the data returned must be 4KB aligned for both the offset and length
> unless the returned range ends on the end of the file, in which case the length
> is not required to be 4KB aligned if the range ends on or beyond the end of the
> file."

So "4KB" alignment applies to BOTH Offset and Length, and the EoF exemption
applies to Length only (Offset must always be 4KB-aligned). "EoF" = the
**logical file size** of the placeholder. **Confirmed exactly as W1 states.**

**Crate `write_at` does NO alignment/chunking — passes raw.** `ticket.rs:64-78`:

```rust
// impl utility::WriteAt for FetchData
/// The buffer passed must be 4KiB in length or end on the logical file size. Unfortunately,
/// this is a restriction of the operating system.
fn write_at(&self, buf: &[u8], offset: u64) -> core::Result<()> {
    command::Write { buffer: buf, position: offset }
        .execute(self.connection_key, self.transfer_key)
}
```

And `command/commands.rs:73-84` (`impl Command for Write`):

```rust
TransferData: CF_OPERATION_PARAMETERS_0_6 {
    Flags: CloudFilters::CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
    CompletionStatus: Foundation::STATUS_SUCCESS,
    Buffer: self.buffer.as_ptr() as *mut _,
    Offset: self.position as i64,     // == offset arg, verbatim
    Length: self.buffer.len() as i64, // == buf.len(), verbatim
}
```

No rounding, no loop, no knowledge of logical file size — single `CfExecute`.
The crate's own doc-comment (`ticket.rs:67-68`) pushes the 4KiB constraint onto
the caller. Our `fetch_data` (cfprovider.rs:84-88) passes `range.start` and
`buf.len()` straight through — no alignment.

**Correction / nuance to add (W1 was slightly imprecise):** W1's per-field gloss
("*Offset* must be 4KB aligned") is correct, but note the EoF exemption in the MS
text covers **Length only**; Offset must always be 4KB-aligned with no EoF escape.
W1's BUG write-up does say "align `start` down and `end` up," which is correct, so
no functional error — just worth stating that a non-aligned *Offset* is never
legal even at EoF. CONFIRMED.

---

## B. `PlaceholderFile::blob()` asserts length ≤ 4096 and PANICS inside the FFI callback if exceeded; no `catch_unwind` in the proxy → UB across `extern "system"`.

**Verdict: CONFIRMED.**

**The assert** — `placeholder_file.rs:78-84` (note: file is at crate `src/` root,
NOT `src/filter/` as some W1 paths imply — see correction below):

```rust
pub fn blob(mut self, blob: Vec<u8>) -> Self {
    assert!(
        blob.len() <= CloudFilters::CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH as usize,
        "blob size must not exceed {} bytes, got {} bytes",
        CloudFilters::CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH,
        blob.len()
    );
```

**The max constant value = 4096 bytes ("4KB").** CF_PLACEHOLDER_CREATE_INFO,
`FileIdentity` field
(<https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_placeholder_create_info>):

> "The *FileIdentity* blob should not exceed **CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH**
> (defined to 4KB) in size. *FileIdentity* gets passed back to the sync provider in
> all callbacks. This is required for files (not for directories)."

`.blob()` is called from our `fetch_placeholders` (cfprovider.rs:123), which the
crate invokes from the `extern "system"` proxy `fetch_placeholders`
(`proxy.rs:135`).

**No `catch_unwind` in the proxy.** Confirmed by reading the entire `proxy.rs`:
every `pub unsafe extern "system" fn ...<T>(info, params)` body directly calls the
trait method with no panic guard. Signatures (all identical shape), e.g.:

```rust
pub unsafe extern "system" fn fetch_placeholders<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) { ... filter.fetch_placeholders(...) ... }   // proxy.rs:135-155

pub unsafe extern "system" fn fetch_data<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) { ... filter.fetch_data(...) ... }            // proxy.rs:79-99
```

There is no `std::panic::catch_unwind` anywhere in `proxy.rs` (grep-confirmed: the
string does not appear). With the default `panic = "unwind"` strategy a panic from
inside `.blob()` unwinds out of the trait method, across the `extern "system"`
boundary into the OS CfAPI dispatcher = undefined behavior (Rust aborts on a
panic that tries to cross an `extern` frame on current toolchains; either way it is
not a recoverable per-entry error). CONFIRMED.

**Correction to W1 file paths (minor, non-substantive):** W1 (`w1_placeholders.md`,
`w1_crate_map.md`) cites the assert as `placeholder_file.rs:79-84`, which is
correct, but `w1_placeholders.md` line 19 lists it under "`placeholder_file.rs`"
in the `filter/` group. The actual path is
`cloud-filter-0.0.6/src/placeholder_file.rs` (crate root), not
`src/filter/placeholder_file.rs`. The line numbers and content are right; only the
directory in the prose grouping is off. Does not affect the verdict.

---

## C. Under HydrationType::Full, transferred data must reach the declared placeholder logical size, so a Google-Docs export whose byte count ≠ the list_dir-reported size will hang/fail hydration.

**Verdict: PARTIAL (real bug, but the stated mechanism is imprecise; the true
failure is the 4KB/EoF alignment contract, not a documented "must reach declared
size under Full" rule).**

What the primary sources DO establish:

1. **The alignment/EoF rule is keyed to the logical (declared) file size.** From A:
   a non-final TRANSFER_DATA must be 4KB-aligned in both offset and length; the
   length-alignment exemption only applies when "the resulting range ends on or
   beyond the logical file size" (CF_OPERATION_PARAMETERS, TransferData). The
   logical file size is what we declare at placeholder creation via
   `Metadata::file().size(m.size)` (cfprovider.rs:108) → `CF_FS_METADATA.FileSize`.
   CF_PLACEHOLDER_CREATE_INFO, `FsMetadata`: "File system metadata to be created
   with the placeholder, including all timestamps, file attributes and file size."

2. **Therefore size≠content genuinely breaks the transfer in the realistic case.**
   If the backend delivers FEWER bytes than the declared `m.size` (Google-Docs
   export to .docx/.pdf is a different length than the Drive-reported size, and
   `download_name` even changes the extension — cfprovider.rs:115-119), our single
   `write_at(&buf, range.start)` produces a buffer whose end is `range.start +
   buf.len()`. For the common full-file fetch `range.start = 0`, so the range ends
   at `buf.len() < m.size`. That end is (a) below the logical file size, so the
   EoF length-exemption does NOT apply, and (b) almost never a 4096 multiple → the
   write is rejected with **STATUS_CLOUD_FILE_INVALID_REQUEST / 0x8007017C**, OR
   (if coincidentally aligned) it succeeds but the required range past `buf.len()`
   is never satisfied and the user I/O stalls to the 60s timeout. If the backend
   delivers MORE bytes than `m.size`, writing past the declared logical size is
   likewise outside the contract. **So the underlying concern is real and the code
   fix (reconcile declared size with delivered bytes) is justified.**

What the primary sources DO **NOT** establish (where W1's wording is overstated):

3. **There is no documented rule that "under Full hydration the transfer must
   reach the declared logical size or it hangs."** The OS asks only for the
   `Required` range — CF_CALLBACK_PARAMETERS, FetchData.RequiredFileOffset:
   "The offset, in bytes, for specifying the range of file data that is absolutely
   needed by the filter in order to satisfy outstanding I/O requests." Hydration
   completion is driven by satisfying the *required range of the triggering I/O*,
   not by reaching the declared file size per se. Partial transfers are explicitly
   allowed: "There is no requirement for the sync provider to return all the data
   required at once" (same page). So "must reach the declared placeholder logical
   size" is true only insofar as the triggering read covers the whole file (the
   common open-the-file case) AND the alignment/EoF rule forces the final range to
   reach/exceed the logical size. It is the **alignment contract** (A), keyed to
   the logical size, that bites — not a standalone "Full requires full size" rule.

4. **The policy that DOES fail outright on incomplete hydration is `AlwaysFull`,
   which we do NOT use.** StorageProviderHydrationPolicy / the CF docs:
   "If this is selected [AlwaysFull] and a placeholder cannot be fully hydrated,
   the platform will fail with ERROR_CLOUD_FILE_INVALID_REQUEST." We register
   `HydrationType::Full` (cfprovider.rs:204), which maps to
   `StorageProviderHydrationPolicy::Full` (sync_root_info.rs:397), NOT AlwaysFull.
   So W1's attribution of the hang to "HydrationType::Full ... must reach the
   declared placeholder logical size" conflates the Full vs AlwaysFull semantics a
   little; the actual enforcement mechanism for our `Full` root is the
   TRANSFER_DATA alignment/EoF rule above, plus the 60s callback timeout for an
   unsatisfied required range (CF_CALLBACK_TYPE remarks; CF_CALLBACK_CANCEL_FLAG_IO_TIMEOUT).

**Net:** the *conclusion* (a Google-Docs export whose byte count ≠ declared size
will fail/stall hydration, and this needs a code fix) is **CONFIRMED as a real
risk**. The *stated cause* ("Full requires the transfer to reach the declared
logical size") is **imprecise** — the precise cause is (i) the 4KB-or-EoF
alignment requirement measured against the declared logical size, and (ii) the
required-range/60s-timeout contract. Hence PARTIAL.

**What would settle the residual uncertainty (whether a short transfer hangs vs.
hard-errors):** a live test on Windows. Create a placeholder declaring `size = N`,
then in `fetch_data` `write_at` a buffer of length `M`:
- Case M < N, M not a multiple of 4096, offset 0 → expect `CfExecute` returns
  `0x8007017C` (HRESULT_FROM_WIN32(ERROR_CLOUD_FILE_INVALID_REQUEST)); `write_at`
  returns Err → user open fails.
- Case M < N, M a multiple of 4096, offset 0 → expect `CfExecute` succeeds but the
  range `M..N` stays unhydrated; the read of bytes ≥ M stalls and the OS issues
  CANCEL_FETCH_DATA with IO_TIMEOUT at 60s.
- Case M > N → write past declared logical size; observe whether platform rejects
  or silently extends the file.
Recommend running this matrix to lock the exact behavior before/after the fix.

---

## D. `pass_with_placeholder` hardcodes DISABLE_ON_DEMAND_POPULATION + PlaceholderTotalCount=len, so one fetch_placeholders call must return ALL children or the rest are lost.

**Verdict: CONFIRMED.**

`command/commands.rs:170-188` (`impl Command for CreatePlaceholders`):

```rust
TransferPlaceholders: CF_OPERATION_PARAMETERS_0_7 {
    // TODO: this flag tells the system there are no more placeholders in this directory (when that can be untrue)
    //       in the future, implement streaming
    Flags: CloudFilters::CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION, // :175
    CompletionStatus: Foundation::STATUS_SUCCESS,
    PlaceholderTotalCount: self.total as i64,                                                  // :177
    PlaceholderArray: <ptr, or null if empty>,
    PlaceholderCount: self.placeholders.len() as u32,
    EntriesProcessed: 0,
}
```

And `total` is fixed to the slice length in `ticket.rs:148-154`:

```rust
pub fn pass_with_placeholder(&self, placeholders: &mut [PlaceholderFile]) -> core::Result<()> {
    command::CreatePlaceholders {
        total: placeholders.len() as _,   // PlaceholderTotalCount = len, every call
        placeholders,
    }
    .execute(self.connection_key, self.transfer_key)
}
```

The flag is **unconditional** on the success path (no parameter, no
override) and the crate's own `// TODO` comment (`commands.rs:173-174`) admits it
asserts "there are no more placeholders in this directory (when that can be
untrue)." Effect confirmed: a single `fetch_placeholders` must enumerate the
entire directory; anything omitted is permanently invisible (no repeat callback).
Our code returns the full `list_dir` in one shot (cfprovider.rs:99-129), so we are
correct today but fragile. CONFIRMED exactly as W1 states.

---

## E. Several `with_*` setters panic via `.unwrap()` on WinRT error; only `with_path`/`with_recycle_bin_uri` return Result.

**Verdict: CONFIRMED.**

From `root/sync_root_info.rs`, each setter quoted:

- `with_hydration_type` (`:254-257`) → `set_hydration_type` (`:249-251`):
  `self.0.SetHydrationPolicy(hydration_type.into()).unwrap();` → returns `Self`.
- `with_population_type` (`:178-181`) → `set_population_type` (`:173-175`):
  `self.0.SetPopulationPolicy(population_type.into()).unwrap();` → returns `Self`.
- `with_version` (`:196-199`) → `set_version` (`:189-193`):
  `self.0.SetVersion(&...to_hstring()).unwrap()` → returns `Self`.
- `with_icon` (`:299-302`) → `set_icon` (`:290-294`):
  `self.0.SetIconResource(&...to_hstring()).unwrap();` → returns `Self`.
- `with_display_name` (`:83-86`) → `set_display_name` (`:76-80`):
  `self.0.SetDisplayNameResource(&...to_hstring()).unwrap()` → returns `Self`.

All five return `Self` and `.unwrap()` the WinRT `Result` internally → panic on
WinRT error.

The two that return `Result`:

- `with_path` (`:162-165`) → `Result<Self>`; the fallible part is
  `GetFolderFromPathAsync(...).unwrap().get()?` then `SetPath(...).unwrap()`
  (`:146-157`). So it returns `Err` if the folder doesn't resolve, but still
  `.unwrap()`s the inner `SetPath`.
- `with_recycle_bin_uri` (`:112-115`) → `Result<Self>`; fallible part is
  `Uri::CreateUri(...)?` (`:99-107`); `SetRecycleBinUri(...).unwrap()` after.

CONFIRMED. Note `default()` (`:335-339`) also `.unwrap()`s
`StorageProviderSyncRootInfo::new()`. Matches W1.

---

## F. The crate registers 13 callbacks and rename/delete/dehydrate/etc default to `Err(NotSupported)`.

**Verdict: CONFIRMED.**

**Callback table** — `proxy.rs:14` `pub type Callbacks = [CF_CALLBACK_REGISTRATION; 14];`
and `callbacks!()` (`proxy.rs:34-77`) registers exactly **13 real callbacks + 1
`CF_CALLBACK_TYPE_NONE` terminator** (the macro appends the terminator at
`:25-28`). The 13, in order:
`FETCH_DATA`, `VALIDATE_DATA`, `CANCEL_FETCH_DATA`, `FETCH_PLACEHOLDERS`,
`CANCEL_FETCH_PLACEHOLDERS`, `NOTIFY_FILE_OPEN_COMPLETION`,
`NOTIFY_FILE_CLOSE_COMPLETION`, `NOTIFY_DEHYDRATE`, `NOTIFY_DEHYDRATE_COMPLETION`,
`NOTIFY_DELETE`, `NOTIFY_DELETE_COMPLETION`, `NOTIFY_RENAME`,
`NOTIFY_RENAME_COMPLETION`. CONFIRMED ("13 callbacks").

**Default trait bodies** — `filter/sync_filter.rs`, the fallible callbacks default
to `Err(CloudErrorKind::NotSupported)`:

- `validate_data` (`:33-40`): `Err(CloudErrorKind::NotSupported)`
- `fetch_placeholders` (`:44-51`): `Err(CloudErrorKind::NotSupported)`
- `dehydrate` (`:69-76`): `Err(CloudErrorKind::NotSupported)`
- `delete` (`:85-92`): `Err(CloudErrorKind::NotSupported)`
- `rename` (`:104-111`): `Err(CloudErrorKind::NotSupported)`

`fetch_data` (`:15-20`) has NO default body — it is a required trait method. The
non-fallible notification callbacks (`cancel_fetch_data` `:23`,
`cancel_fetch_placeholders` `:54`, `opened` `:58`, `closed` `:62`, `dehydrated`
`:79`, `deleted` `:95`, `renamed` `:114`, `state_changed` `:124`) default to empty
`{}` no-ops. `NotSupported` → `STATUS_CLOUD_FILE_NOT_SUPPORTED` (error.rs:100). We
override only `fetch_data` + `fetch_placeholders`, so delete/rename/dehydrate of
placeholders are refused by default. CONFIRMED exactly as W1 states.

---

## G. Sync root Id assembled string max length is 174 chars; provider_id portion otherwise allows up to 255 in the crate assert.

**Verdict: CONFIRMED.**

**174-char total-Id limit** — StorageProviderSyncRootInfo.Id, Remarks
(<https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageprovidersyncrootinfo.id>),
verbatim:

> "An identifier in the form: [Storage Provider ID]![Windows SID]![Account ID]"
> "An example of an ID might look something like: \"OneDrive!S-1-1234!Personal\"."
> "Note that the maximum allowed length for an ID is 174 characters. Setting a
> longer ID can result in an error (ERROR_INSUFFICIENT_BUFFER)."

**255-char provider-name assert in the crate** — `root/sync_root_id.rs:59-71`
(`SyncRootIdBuilder::new`):

```rust
assert!(
    name.len() <= CloudFilters::CF_MAX_PROVIDER_NAME_LENGTH as usize,
    "provider name must not exceed {} characters, got {} characters",
    CloudFilters::CF_MAX_PROVIDER_NAME_LENGTH,
    name.len()
);
```

`CF_MAX_PROVIDER_NAME_LENGTH = 255` — CF_SYNC_ROOT_PROVIDER_INFO
(<https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_sync_root_provider_info>):

> "*ProviderName* is an end-user facing string with a maximum length of
> **CF_MAX_PROVIDER_NAME_LENGTH** (255 characters)."

So the crate gate is 255 on the **provider-name component**, while the MS limit on
the **assembled Id** (`provider!SID!account`) is 174. The crate has no
assembled-length check — `build()` (`sync_root_id.rs:99-109`) just joins the three
components with `!` (SEPARATOR = `0x21`, `:126`) into an HSTRING. Therefore a long
label can pass the crate's 255-char provider assert yet exceed the 174-char Id
limit and fail `Register` with `ERROR_INSUFFICIENT_BUFFER`. CONFIRMED exactly as
W1 states. (The assert is a panic, not a `Result`, confirming W1's separate point
that a >255-char provider name panics rather than erroring gracefully.)

---

## Summary of corrections / overstatements found

- **C is PARTIAL, not a clean CONFIRMED.** The real-world failure (Google-Docs
  export size ≠ declared size breaks hydration) is genuine and the code fix is
  warranted, but W1's stated *mechanism* ("HydrationType::Full requires the
  transfer to reach the declared placeholder logical size") is imprecise. The
  precise enforced contract is the TRANSFER_DATA 4KB-or-EoF alignment rule
  (measured against the declared logical size) plus the required-range/60s-timeout
  rule. The policy that hard-fails on incomplete hydration is **AlwaysFull**, which
  we do NOT register; we register **Full**. This distinction should be reflected in
  the fix's rationale.
- **A: minor precision.** Microsoft's EoF exemption applies to **Length only**;
  **Offset must always be 4KB-aligned** (no EoF escape). W1's gloss occasionally
  reads as if both are exempt at EoF. The W1 fix recommendation (floor start, ceil
  end) is nonetheless correct.
- **B: file-path nit.** The blob assert is in
  `cloud-filter-0.0.6/src/placeholder_file.rs` (crate root), not under
  `src/filter/`. Line numbers and content in W1 are correct; the directory grouping
  in `w1_placeholders.md` prose is slightly off. No substantive impact.
- **D, E, F, G: CONFIRMED with no corrections.** Quotes match crate source and MS
  docs verbatim.

No claim was found to be outright WRONG (no REFUTED verdicts). One claim (C) is
overstated in its mechanism; two (A, B) have minor imprecisions that do not change
the conclusions or recommended fixes.

---

### Primary sources used

- CF_OPERATION_PARAMETERS (TransferData Offset/Length 4KB + EoF; CompletionStatus):
  <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_operation_parameters>
- CF_CALLBACK_PARAMETERS (FetchData Required/Optional ranges; 4KB rule; no-clobber):
  <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_callback_parameters>
- CF_PLACEHOLDER_CREATE_INFO (FsMetadata file size; FileIdentity 4KB max):
  <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_placeholder_create_info>
- StorageProviderSyncRootInfo.Id (174-char limit, ERROR_INSUFFICIENT_BUFFER):
  <https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageprovidersyncrootinfo.id>
- CF_SYNC_ROOT_PROVIDER_INFO (CF_MAX_PROVIDER_NAME_LENGTH = 255):
  <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_sync_root_provider_info>
- StorageProviderHydrationPolicy (AlwaysFull fails with ERROR_CLOUD_FILE_INVALID_REQUEST):
  <https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageproviderhydrationpolicy>
- Crate source (v0.0.6): `src/filter/ticket.rs`, `src/command/commands.rs`,
  `src/placeholder_file.rs`, `src/filter/proxy.rs`, `src/filter/sync_filter.rs`,
  `src/filter/info.rs`, `src/root/sync_root_info.rs`, `src/root/sync_root_id.rs`,
  `src/error.rs`
