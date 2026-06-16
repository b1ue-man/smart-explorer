# W1 — File Hydration (FETCH_DATA / TRANSFER_DATA) review

Audit of Smart Explorer's native CfAPI hydration path against the `cloud-filter`
v0.0.6 crate source and the Microsoft CfAPI documentation.

- **Our code:** `/home/user/smart-explorer/native/src/cfprovider.rs`,
  `impl SyncFilter for RemoteFilter::fetch_data` (lines 58–90).
- **Crate (ground truth):** `cloud-filter-0.0.6` registry source (paths quoted inline).
- **Docs:** Microsoft Learn `cfapi.h` reference (URLs cited inline).
- Date of review: 2026-06-16. Crate version: 0.0.6.

---

## (a) The exact byte/offset contract CfAPI expects (quoted)

### The TRANSFER_DATA alignment rule — the central contract

From **CF_OPERATION_PARAMETERS**, the `TransferData` member
(<https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_operation_parameters>):

> *OpParams.TransferData.Offset* and *OpParams.TransferData.Length* describe a
> range in the placeholder to which the sync provider is transferring the data.
> There is no requirement that the sync provider return all data as requested in
> one shot. It is also OK for a sync provider to return more data than requested.
> ... The only requirement is that **both offset and length are 4KB aligned
> unless the range described ends on the logical file size (EoF), in which case,
> the length is not required to be 4KB aligned as long as the resulting range
> ends on or beyond the logical file size.**

Per-field, same page:

> `TransferData.Offset` — ... ***Offset* must be 4KB aligned.**
>
> `TransferData.Length` — The *Length* in bytes of the *Buffer*. **The length
> must be 4KB aligned unless the range described ends on the logical file size
> (EoF), in which case, the *Length* is not required to be 4KB aligned as long as
> the resulting range ends on or beyond the logical file size.** Even if the
> *CompletionStatus* is not **STATUS_SUCCESS**, this field should be set to a
> valid value.

The same rule is restated from the requesting side, in **CF_CALLBACK_PARAMETERS**
under `FetchData` (<https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_callback_parameters>):

> There is no requirement for the sync provider to return all the data required
> at once. There is also no requirement for the sync provider to return the data
> within either the required range or optional range. The platform ensures under
> no circumstances will modified/unsynced file data get clobbered because of an
> invalid **CF_OPERATION_TYPE_TRANSFER_DATA** operation. **However, the data
> returned must be 4KB aligned for both the offset and length unless the returned
> range ends on the end of the file, in which case the length is not required to
> be 4KB aligned if the range ends on or beyond the end of the file.**

So "4KB" = **4096 bytes**. EoF here means the **logical (placeholder) file size**,
i.e. the size we set on the placeholder in `fetch_placeholders`
(`Metadata::file().size(m.size)`, cfprovider.rs:107), *not* whatever the backend
stream happens to return.

### Buffer requirement

> `TransferData.Buffer` — A valid user mode buffer. This must point to a valid
> user mode buffer if *CompletionStatus* is **STATUS_SUCCESS** and **should be of
> at least *Length* bytes.** Otherwise, the buffer field is ignored.

### What the OS asks us for (required vs optional)

From `FetchData` (same callback-parameters page):

> `RequiredFileOffset` — The offset, in bytes, for specifying the range of file
> data that is **absolutely needed** by the filter in order to satisfy
> outstanding I/O requests.
>
> `RequiredLength` — The length, in bytes, of the file data that is **absolutely
> needed** ...
>
> `OptionalFileOffset` / `OptionalLength` — ... a **hint** as to a broader range
> of file data that could usefully be given to the platform, in case the sync
> provider prefers to give data in larger chunks. **Usually the optional range
> will be the maximal contiguous range that is not currently present in the
> placeholder.** ... A length of -1, denoted as `CF_EOF`, means infinity (i.e.
> to end of file).

The crate surfaces these in info.rs:

```
// info.rs:29-32
pub fn required_file_range(&self) -> Range<u64> {
    (self.0.RequiredFileOffset as u64)
        ..(self.0.RequiredFileOffset + self.0.RequiredLength) as u64
}
```

### Error/ack semantics on completion

> In the successful transfer case, any pending user IO requests [o]n the
> placeholder file that have received all needed bytes as a result of the
> transfer will be completed; otherwise the incomplete user IO requests will be
> updated to reflect the latest hydration state. **In a failed transfer case, any
> pending user IO requests on the placeholder file that overlap with the range as
> described by the offset and length will be failed with
> *OpParams.TransferData.CompletionStatus*.**
> (CF_OPERATION_PARAMETERS, TransferData)

> *OpParams.TransferData.CompletionStatus* must be set to **STATUS_SUCCESS** ...
> If the sync provider fails ... it must set a **STATUS_CLOUD_FILE_*** status ...
> **Any status code returned outside of STATUS_CLOUD_FILE_* range except for
> STATUS_SUCCESS will be converted to STATUS_CLOUD_FILE_UNSUCCESSFUL by the
> platform.**

### Timeout / threading (CfExecute + callback type)

From **CfExecute** (<https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfexecute>):

> A valid call to **CfExecute** will reset the timers of all pending callback
> requests that belong to the same sync provider process.
>
> All operations can be performed in an arbitrary thread context in the sync
> provider process.

From **CF_CALLBACK_TYPE** (<https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type>):

> Callback routines will be invoked in an **arbitrary thread (part of a thread
> pool)**. Multiple callbacks can occur simultaneously, in different threads, and
> it is the responsibility of the sync provider code to implement any necessary
> synchronization ... **Every callback request has a fixed 60 second timeout. A
> valid operation on any pending requests from the sync provider resets the
> timers of all pending requests.** ... Callback routines have no return value.

`CF_CALLBACK_TYPE_FETCH_DATA` itself:

> This callback is used to ask the sync provider for a range of file data that is
> required in order to satisfy an I/O request, or an explicit hydration request,
> on a placeholder. Implementation of this callback is required if the sync
> provider specifies a hydration policy that is *not* **ALWAYS_FULL** ...

(Note: we register `HydrationType::Full`, cfprovider.rs:205 — which is *not*
`ALWAYS_FULL`, so FETCH_DATA is indeed required and will fire.)

---

## (b) What the crate's `write_at` actually does

The ticket method we call (`ticket.write_at(&buf, range.start)`):

```
// ticket.rs:64-78  — impl utility::WriteAt for FetchData
/// The buffer passed must be 4KiB in length or end on the logical file size. Unfortunately,
/// this is a restriction of the operating system.
///
/// This method is equivalent to calling `CfExecute` with `CF_OPERATION_TYPE_TRANSFER_DATA`.
fn write_at(&self, buf: &[u8], offset: u64) -> core::Result<()> {
    command::Write {
        buffer: buf,
        position: offset,
    }
    .execute(self.connection_key, self.transfer_key)
}
```

And the command it builds:

```
// commands.rs:73-84  — impl Command for Write
fn build(&self) -> CF_OPERATION_PARAMETERS_0 {
    CF_OPERATION_PARAMETERS_0 {
        TransferData: CF_OPERATION_PARAMETERS_0_6 {
            Flags: CloudFilters::CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
            CompletionStatus: Foundation::STATUS_SUCCESS,
            Buffer: self.buffer.as_ptr() as *mut _,
            Offset: self.position as i64,        // == range.start, passed straight through
            Length: self.buffer.len() as i64,    // == buf.len(), passed straight through
        },
    }
}
```

Then `executor::execute` calls `CfExecute` once (executor.rs:56–82), with
`ParamSize` computed as `size_of::<Field>() + offset_of!(.., Anonymous)`
(executor.rs:74–76) — which matches the docs requirement
("*OpParams.ParamSize* must be set to the exact size of *OpParams.TransferData*
plus the offset of *OpParams.TransferData*").

**Conclusions about `write_at`:**

- **Single shot.** One `buf`, one `CfExecute`. No internal loop, no chunking.
- **NOT aligned.** `Offset` and `Length` are passed through verbatim from
  whatever we hand it. There is **no rounding to 4096** anywhere in
  `write_at` → `Write::build` → `execute`. The crate's own doc-comment
  (ticket.rs:67-68) explicitly pushes the 4KiB constraint onto the **caller**:
  *"The buffer passed must be 4KiB in length or end on the logical file size."*
- **No EoF padding.** It does not pad the final chunk up to a 4KB boundary, nor
  does it know the logical file size. Correctness at EoF depends entirely on the
  buffer we pass actually reaching/exceeding the logical file size.

So **all alignment responsibility is ours**, and our `fetch_data` does **no
alignment at all** — it passes `buf = bytes[range.start .. range.end]` and
`offset = range.start` directly (cfprovider.rs:84-88).

---

## (c) Answers to the 5 critical questions

### Q1. Does CfAPI require Offset/Length aligned (except at EoF)? Will our `write_at` fail, with what HRESULT? Does the crate align internally?

**Yes, CfAPI requires 4096-byte alignment of both Offset and Length, except when
the range ends at/after the logical file size.** Quoted in (a):
*"both offset and length are 4KB aligned unless the range described ends on the
logical file size (EoF)..."* (CF_OPERATION_PARAMETERS, TransferData).

**The crate does NOT align internally** (see (b)). It forwards our raw
`range.start` and `buf.len()` to `CfExecute`.

**Will it fail?** If the OS hands us a `required_file_range` whose `start` or
`length` is not a multiple of 4096 *and* the range does not extend to EoF, then
`CfExecute(TRANSFER_DATA)` returns an error and `write_at` returns `Err`. The
documented/observed HRESULT is **`0x8007017C` — "The cloud operation is invalid"**
(`ERROR_CLOUD_FILE_INVALID_REQUEST` / `STATUS_CLOUD_FILE_INVALID_REQUEST`).
Microsoft Q&A, "Windows 10 File Cloud / Sync Provider API - TransferData problem"
(<https://learn.microsoft.com/en-us/answers/questions/353466/windows-10-file-cloud-sync-provider-api-transferda>):

> "If I comment out the `if (0 != len % 4096) len = 4096 * (len / 4096 + 1);`
> part then I am getting the dreaded **0x8007017c the cloud operation is invalid**
> ..."

**In practice today, how exposed are we?** The required range the OS supplies is
itself a *file range* derived from the I/O that triggered hydration. Empirically
for a full hydration of a dehydrated file the OS supplies a required range that
starts at 0 and runs to (at least) EoF, so `offset = 0` (aligned) and the buffer
reaches the logical size (EoF exemption applies) — that path works, which is why
this code functions in the common "open the whole file" case. The danger is the
**partial / mid-file** case: a memory-mapped read or a ranged read can produce a
required range starting at a non-4096 offset, or ending mid-file on a non-4096
length. In that case our un-padded, un-aligned `write_at` will be rejected with
`0x8007017C`. **The crate's own doc-comment warns us this is our job, and we are
not doing it.** → see ISSUES (BUG).

### Q2. Is `required_file_range()` guaranteed aligned by the OS? Is serving exactly the required range sufficient, or can the OS request the whole file?

**The required range is NOT guaranteed to be 4096-aligned.** The docs describe it
only as *"the range of file data that is absolutely needed ... in order to satisfy
outstanding I/O requests"* (CF_CALLBACK_PARAMETERS, FetchData.RequiredFileOffset/
RequiredLength). Required offset/length track the triggering I/O, which can be any
byte range. The 4KB alignment constraint is placed on **what we transfer back**,
not on what the OS requests — that is precisely why the provider is expected to
*round outward* (down on offset, up on length) before calling TRANSFER_DATA.

**Serving exactly the required range is not sufficient in general.** Two reasons:

1. **Alignment** — exact required bounds may be mid-page (Q1).
2. **The OS can and does request large/whole-file ranges.** For a normal "open
   the file" hydration the required range typically spans 0..EoF (this is the
   common path that makes our code work). The `OptionalFileOffset/OptionalLength`
   field exists specifically to hint a *broader* range —
   *"Usually the optional range will be the maximal contiguous range that is not
   currently present in the placeholder"* — and may be `CF_EOF` (-1) meaning to
   end of file. We currently **ignore the optional range entirely**
   (cfprovider.rs only reads `required_file_range`, line 70).

The safe contract is: clamp/expand the range to `[floor(start, 4096) ..
min(ceil(end, 4096), logical_size)]`, fill the buffer for that aligned range, and
transfer at the aligned offset — letting the EoF exemption cover the tail.

### Q3. Short read: `open_read` returns fewer bytes than `len`. Does CfAPI error or hang?

Our code (cfprovider.rs:85-88):

```
let len = range.end.saturating_sub(range.start);
let mut buf = Vec::new();
r.take(len).read_to_end(&mut buf).map_err(cerr)?;
ticket.write_at(&buf, range.start).map_err(cerr)?;
```

`read_to_end` returns whatever the stream yields up to `len`. If the backend
returns fewer bytes than `len` (stream EoF earlier than the placeholder's logical
size, e.g. the placeholder size we set in `fetch_placeholders` is stale/too big),
then `buf.len() < len` and we call `write_at(&buf, range.start)` with a **short
buffer that does not reach the logical file size**.

Consequences, per the docs:

- The buffer is then **neither 4KB-length-aligned nor EoF-reaching** (unless it
  coincidentally lands on a 4KB multiple), so `CfExecute` is likely rejected with
  `0x8007017C` (Q1) → we return `Err` → the failed-transfer path runs.
- Even if it *were* accepted, *"the incomplete user IO requests will be updated to
  reflect the latest hydration state"* (CF_OPERATION_PARAMETERS, TransferData) —
  i.e. the bytes we *did* transfer are committed, but the still-missing tail of the
  required range remains unhydrated. The OS does **not** hang on our process: each
  callback has the *"fixed 60 second timeout"* (CF_CALLBACK_TYPE remarks); if we
  never satisfy the required range, the *user's* I/O is what stalls/fails, and the
  request is cancelled at timeout (surfacing later as a `CANCEL_FETCH_DATA` with
  `CF_CALLBACK_CANCEL_FLAG_IO_TIMEOUT`). So: **CfAPI does not hang the OS, but a
  persistent short read leaves the user's read failing/looping and ultimately
  timing out.** Partial transfers are explicitly *allowed*, but you must
  eventually deliver the whole required range (possibly via multiple
  TRANSFER_DATA calls); a permanent shortfall is an error condition, not a
  supported steady state.

We also do **not** verify `buf.len() == len`, and we do **not** report progress
between chunks, so a legitimately huge-but-slow backend read can itself blow the
60s timeout (Q4). → see ISSUES.

### Q4. Memory: we buffer the entire required range in RAM. Problem for multi-GB? Does the OS request the whole file at once or chunk it?

**Yes — this is a real memory/latency risk.** We do `let mut buf = Vec::new();
r.take(len).read_to_end(&mut buf)` (cfprovider.rs:86-87), which **allocates and
holds the entire required range in a single heap `Vec`** before any byte is handed
to the OS.

- The OS *commonly requests the whole file as one required range* for a plain
  open of a dehydrated file (this is the path that works today). So for a 4 GB
  file, `len ≈ 4 GB` and we attempt a single 4 GB allocation + full download into
  RAM before the first `write_at`. That risks OOM / allocation failure and means
  **zero bytes are delivered until the entire file is downloaded**, which for any
  non-trivial file will exceed the **60-second callback timeout** (no
  `report_progress`/`CfReportProviderProgress` is called to reset it —
  ticket.rs:35 exists but we never call it).
- The reference design is the opposite: Microsoft's Cloud Mirror sample
  transfers in fixed **`CHUNKSIZE` (4096)**-based chunks in a loop, advancing
  `StartOffset`/`RemainingLength` per chunk
  (`Windows-classic-samples/Samples/CloudMirror`), precisely so it streams first
  bytes fast, bounds memory, and keeps resetting the timeout. The docs sanction
  this: *"There is no requirement that the sync provider return all data as
  requested in one shot ... The sync provider can also perform multiple
  TRANSFER_DATA operations repeatedly as a response to the same FETCH_DATA
  callback."* (CF_OPERATION_PARAMETERS, TransferData). CloudMirror issue #143
  ("CloudMirror can't hydrate files greater than 1GB") is the canonical example
  that whole-file-in-one-buffer approaches break on large files.

So: chunk it. Buffer at most one aligned chunk (e.g. a few MB, multiple of 4096),
`write_at` it at its aligned offset, advance, repeat, and call
`report_progress` each chunk. → see ISSUES (RISK/FIDELITY).

### Q5. Error path: `Err -> CloudErrorKind::Unsuccessful`. How does proxy.rs translate it to the OS? Does a failed fetch leave the file in a bad state?

Our error mapper collapses *every* failure to one kind (cfprovider.rs:29-31):

```
fn cerr<E: std::fmt::Display>(_e: E) -> CloudErrorKind { CloudErrorKind::Unsuccessful }
```

The proxy turns a returned `Err(e)` into a *failed* TRANSFER_DATA via the
`Fallible` path (proxy.rs:79-99):

```
let Err(e) = filter.fetch_data(request, ticket, info::FetchData(...)) else {
    return;
};
command::Write::fail(connection_key, transfer_key, e).unwrap();
```

`Write::fail` issues another `CfExecute(TRANSFER_DATA)` carrying the error status
(commands.rs:87-109):

```
TransferData: CF_OPERATION_PARAMETERS_0_6 {
    Flags: CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
    CompletionStatus: error_kind.into(),  // STATUS_CLOUD_FILE_UNSUCCESSFUL
    Buffer: [0;1].as_mut_ptr() as *mut _,
    Offset: 0,
    Length: 0,
}
```

`CloudErrorKind::Unsuccessful` maps to `STATUS_CLOUD_FILE_UNSUCCESSFUL`
(error.rs:130). Note `Write::fail` sends `Offset:0, Length:0` with a dummy buffer
— this is the *acknowledgement of failure*, and the docs confirm a non-success
`CompletionStatus` is the supported way to decline the hydration (the buffer field
is then *"ignored"*; *"Even if the CompletionStatus is not STATUS_SUCCESS, this
[Length] field should be set to a valid value"* — here 0, which is valid).

**Important crate caveat:** if the *failing* `CfExecute` itself returns an error,
`.unwrap()` (proxy.rs:97) will **panic the OS thread-pool callback thread**. That
is the crate's behavior, not ours, but it means a malformed/already-timed-out
ticket on the error path can crash the callback worker.

**Does a failed fetch leave the file in a bad state?** Per the docs, **no
permanent corruption**: *"The platform ensures under no circumstances will
modified/unsynced file data get clobbered because of an invalid
CF_OPERATION_TYPE_TRANSFER_DATA operation"* (CF_CALLBACK_PARAMETERS, FetchData).
On a failed transfer, *"any pending user IO requests on the placeholder file that
overlap with the range ... will be failed with ... CompletionStatus"*
(CF_OPERATION_PARAMETERS, TransferData) — i.e. the user's open/read fails with
"the cloud operation was unsuccessful," and the file **stays a dehydrated
placeholder**. The next open re-triggers FETCH_DATA cleanly. So a failed fetch is
*recoverable* (retryable), but it is **user-visible**: the file appears to fail to
open. The over-broad mapping to `Unsuccessful` also means real causes (network
down, auth failure, file gone) are all reported identically, producing unhelpful
shell error text and no actionable sync-status blob (we pass `SyncStatus: null`,
executor.rs:67). → see ISSUES.

---

## (d) ISSUES

- **[BUG] No 4096-byte alignment of TRANSFER_DATA offset/length for non-EoF
  ranges.** `fetch_data` passes `offset = range.start` and `buf.len() =
  range.end-range.start` straight to `ticket.write_at` (cfprovider.rs:84-88); the
  crate forwards them unmodified to `CfExecute` (ticket.rs:71-77,
  commands.rs:73-84). If the OS supplies a required range whose start or length is
  not a multiple of 4096 and the range does not reach the logical file size,
  `CfExecute` fails with **`0x8007017C` ERROR_CLOUD_FILE_INVALID_REQUEST**
  (Q1; <https://learn.microsoft.com/en-us/answers/questions/353466/windows-10-file-cloud-sync-provider-api-transferda>),
  and the user's read fails. Fix: align `start` down and `end` up to 4096,
  clamp `end` to the logical file size, and read/write that aligned window. The
  crate's own doc-comment (ticket.rs:67-68) states this is the caller's
  responsibility.

- **[BUG] Short backend read silently produces an invalid/short TRANSFER_DATA.**
  `r.take(len).read_to_end(&mut buf)` (cfprovider.rs:87) does not guarantee
  `buf.len() == len`; a stream shorter than the placeholder's logical size yields
  a buffer that neither hits a 4KB boundary nor reaches EoF, so `write_at` is
  rejected (`0x8007017C`) or leaves the required range unsatisfied, stalling the
  user I/O until the 60s timeout/cancel (Q3; CF_CALLBACK_TYPE remarks,
  CF_OPERATION_PARAMETERS TransferData). The placeholder size we set in
  `fetch_placeholders` (`Metadata::file().size(m.size)`, cfprovider.rs:107) must
  exactly match the bytes the backend will deliver, or be reconciled here. Fix:
  verify the byte count, pad to the placeholder's logical size at EoF, or fail
  with a precise error.

- **[RISK] Whole required range buffered in a single RAM allocation; no
  chunking.** `Vec::new()` + `read_to_end` of the full `len` (cfprovider.rs:86-87)
  means multi-GB files attempt a single multi-GB allocation and download fully
  before the first byte is transferred (Q4). For the common whole-file required
  range this risks OOM and *guarantees* exceeding the 60s timeout for large/slow
  files. Reference design (Cloud Mirror, `CHUNKSIZE 4096`-based loop; docs:
  *"perform multiple TRANSFER_DATA operations repeatedly"*) streams bounded
  chunks. Fix: loop in aligned chunks (e.g. 1–8 MiB, multiple of 4096),
  `write_at` each at its aligned offset.

- **[FIDELITY] No progress reporting; 60s timeout exposure.** We never call
  `ticket.report_progress` / `CfReportProviderProgress` (available at
  ticket.rs:35-46). Per CF_CALLBACK_TYPE, every callback has a fixed 60s timeout
  reset only by a valid CfExecute/progress call. A slow single-shot download with
  no intermediate operations will be cancelled at 60s
  (CF_CALLBACK_CANCEL_FLAG_IO_TIMEOUT). Chunked transfers (above) plus
  `report_progress` per chunk resolve this and give the user a progress UI.

- **[FIDELITY] `optional_file_range` ignored.** We only honor the required range
  (cfprovider.rs:70) and never consult `info.optional_file_range()`
  (info.rs:39-42). The optional range is the OS hint for *"the maximal contiguous
  range that is not currently present in the placeholder"* and is the intended way
  to hydrate efficiently in large chunks (CF_CALLBACK_PARAMETERS, FetchData
  Optional*). Honoring it (clamped to logical size) reduces callback churn. Not a
  correctness bug — the field is explicitly optional — but a behavior gap vs.
  OneDrive-class providers.

- **[FIDELITY] All errors collapse to `CloudErrorKind::Unsuccessful`.** `cerr`
  (cfprovider.rs:29-31) discards the cause; everything becomes
  `STATUS_CLOUD_FILE_UNSUCCESSFUL` (error.rs:130). Network-down, auth-failure,
  not-found, and a genuine alignment bug are indistinguishable to the user, and we
  set no `SyncStatus` blob (executor.rs:67) so the shell shows a generic message.
  Map at least `NetworkUnavailable`, `AuthenticationFailed`, and not-found to the
  matching `CloudErrorKind` variants (all already defined in error.rs:8-72).

- **[RISK] Crate `.unwrap()` on the failure path can panic the callback thread.**
  On our `Err` return, proxy.rs:97 does `command::Write::fail(...).unwrap()`. If
  that error-ack `CfExecute` itself fails (e.g. ticket already timed out), the
  thread-pool callback thread panics. This is crate behavior we inherit, not our
  code, but it means returning `Err` is not always benign. Mitigation on our side:
  prefer to deliver a valid (even if zero-progress) transfer where possible and
  keep `fetch_data` fast enough to avoid timed-out tickets.

- **[OK] Single-shot full-file hydration works because of the EoF exemption.**
  For the common case (open a dehydrated file → required range `0..logical_size`),
  `offset = 0` is 4096-aligned and the buffer reaches the logical file size, so
  the EoF exemption (a) makes the unaligned trailing length legal. This is why the
  current code functions for ordinary opens despite the issues above.

- **[OK] Error-ack and ParamSize accounting are correct.** The `Write::fail` path
  (commands.rs:87-109) sets a non-success `CompletionStatus`
  (`STATUS_CLOUD_FILE_UNSUCCESSFUL`) with `Length:0`, matching the documented
  decline protocol, and `ParamSize` is computed correctly
  (executor.rs:74-76 vs. *"exact size of OpParams.TransferData plus the offset
  of OpParams.TransferData"*). The platform guarantees no data clobbering on a
  failed/invalid transfer, so failures are recoverable on next open
  (CF_CALLBACK_PARAMETERS, FetchData).

---

### Sources

- CF_OPERATION_PARAMETERS (TransferData / alignment): <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_operation_parameters>
- CfExecute (threading, timeout reset): <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfexecute>
- CF_CALLBACK_PARAMETERS (Required/Optional ranges, 4KB rule, no-clobber): <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_callback_parameters>
- CF_CALLBACK_TYPE (FETCH_DATA, thread pool, 60s timeout): <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type>
- 0x8007017C alignment failure + 4096 round-up fix: <https://learn.microsoft.com/en-us/answers/questions/353466/windows-10-file-cloud-sync-provider-api-transferda>
- CloudMirror large-file (>1GB) hydration limitation: <https://github.com/microsoft/Windows-classic-samples/issues/143>
- Crate source: `cloud-filter-0.0.6/src/filter/ticket.rs`, `src/filter/info.rs`, `src/filter/request.rs`, `src/filter/proxy.rs`, `src/command/commands.rs`, `src/command/executor.rs`, `src/error.rs`
