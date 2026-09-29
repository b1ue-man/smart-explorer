# Direct remote opening and recovery

Date: 2026-09-29. Implementation is committed; remote acceptance and publication
are pending. Open work is tracked only in `TODO.md`.

## Goal and evidence

Fix intermittent desktop opening of Android Direct files: premature missing-temp
recovery errors, one pending file blocking another file's manifest, and interrupted
read downloads. Preserve Windows/Linux, Direct/Room identity and authorization,
other remote backends, editor save-back/conflicts, recovery, and staged publication.
No local builds, native commands or tests are permitted.

The reported log contains `editor temp copy is temporarily absent`, refusal to
open without a recovery manifest, and `Datei oeffnen: connection lost`.
Inspection before M1 established:

- `app/os/shared/remote_open.rs::open_file` registers an edit before downloading.
  `poll_remote_edits` observes the absent final filename after 1.5 seconds, before
  checking the download sentinel, and changes its baseline/dirty state.
- `remote_helpers/recovery_manifest.rs::sync_recovery_manifest` validates every
  registered filename, including pending downloads. One absent file blocks the
  manifest needed by a different completed download.
- `transfer/os/shared/local_stage.rs::download_to_id` downloads into a private
  sibling, publishes only after completion, but returns a read failure immediately.
- `share/core/peer_request.rs` already retries read setup; `peer_read.rs` makes
  an interrupted data stream terminal and invalidates only its failed connection
  generation. `node_sessions.rs` reconnects subsequent operations with identity
  checks; five-second keepalives and a twenty-second idle timeout already exist.
- Desktop Share runs through `AgentBackend` and daemon IPC to `PeerBackend`.
  Recovery must therefore retain typed error meaning across that existing boundary.

The log alone cannot establish whether Android suspended, changed networks, lost
radio connectivity, or restarted. A full device reproduction remains distinct
from deterministic remote CI evidence.

## Stage one

1. Model download and editor ownership explicitly. Do not watch or require a
   recovery payload before successful publication. Keep the mandatory durable
   manifest before launching an editor.
2. Preserve an already registered editor copy across atomic delete/rename saves
   without letting that transient absence block unrelated completed opens. Keep
   unsafe paths, links and actual manifest write failures fail-closed.
3. Harden the read-only open download against brief Share transport failures with
   bounded retries through the existing authenticated reconnection path. Never
   replay remote mutations or append unverifiable bytes from a different revision.
4. After implementation, assemble one remote task entrypoint covering these cases
   and their directly affected integrations on Windows and Linux; commit and push,
   run that pipeline, then use the existing terminal release workflow once.

## Research, checked 2026-09-29

[QUIC RFC 9000 §10.1](https://www.rfc-editor.org/rfc/rfc9000.html#section-10.1)
allows a connection to expire after its negotiated idle timeout. Migration and
keepalives do not resurrect terminated application streams. Reuse the existing
authenticated reconnect machinery instead of changing global timeouts.
[Android Doze guidance](https://developer.android.com/training/monitoring-device-state/doze-standby)
documents deferred network access; a foreground service is not a promise of
uninterrupted radio availability. The existing Android background mode must remain
user-controlled.

[Rust `io::copy`](https://doc.rust-lang.org/std/io/fn.copy.html) returns immediately
on read/write failure. A download retry must distinguish remote read failure from
local write failure, close its stage, and discard it before starting again.
[Windows deletion semantics](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-deletefilew)
also require handling open file handles correctly. Keep exclusive stage creation,
flush/sync, and atomic final publication.

Second gap review against the locked `iroh`/`noq` 1.0.1 source confirms that
`ReadError::ConnectionLost` becomes `NotConnected` but displays only `connection
lost`. Daemon `Frame::Err(String)` currently loses that kind. Preserve explicit
transport kinds in a backward-compatible string envelope; leave unrecognized and
ordinary filesystem messages on their existing path. Decode on the agent side
without adding mutation retries. Retain the underlying QUIC cause for diagnosis.

Before this change, `download_to_id` was only consumed by desktop opening. Keep its public
path-only API as a wrapper; a new edit-download result carries the metadata from
the successful attempt, so an unrelated later stat cannot silently adopt a newer
conflict baseline. Respect `read_size` and stable IDs for transformed Drive files
and duplicate names. Whole-file restart avoids mixing revisions; Share metadata
before/after a successful attempt additionally rejects observable source drift.

The manifest-failure path also collected uploads and launched them despite a
failed safety check. M2 retains those edits for retry and suppresses that launch.
M4 also keeps a failed remote stat from bypassing an already known conflict
baseline during save-back; the local edit remains dirty and retryable instead.
After acknowledged publication, a failed or unknown follow-up revision retains
the previous known baseline without replaying the upload. A later save with an
unavailable revision remains local until the conflict check can be completed.
The desktop Share open path wraps its daemon backend in `CachingBackend`.
Revision checks therefore invalidate that browsing cache before Share download
snapshots, after completed reads, and before save-back conflict checks. The same
cached boundary is included in the source-drift and real-peer acceptance cases.

## Stage two: final milestone plan

| Milestone | Affected boundary and dependency | Expected remote acceptance |
| --- | --- | --- |
| M1: editor lifecycle | `remote_helpers/temp.rs`, `remote_helpers.rs`, `remote_open.rs`, recovery manifest; independent | Explicit downloading/downloaded/editing phases. Slow and concurrent downloads produce no false recovery errors or uploads. Only a published regular file with a durable manifest reaches editor launch. |
| M2: retained recovery | Same manifest boundary; depends on M1 | A previously manifested editor file may briefly disappear without blocking another open. Its entry survives with dirty state. Never accept a new absent payload, unsafe parent/link, failed manifest write, or escaped path. Save-back requires stable reappearance. Failed downloads and worker exits clean only their own temp. |
| M3: transport diagnosis | `agent_proto/core/transport_error.rs`, `agent_proto/mod.rs`, daemon `backend_server.rs`, agent `agent_error.rs`, Share `framing.rs`; independent | Transport kinds/message context survive daemon IPC and streamed errors. Legacy/unknown errors and congestion remain compatible. Lost write acknowledgements remain failed and are never replayed. |
| M4: bounded open recovery | `transfer/os/shared/local_stage.rs`, new `edit_download.rs`, transfer re-exports, desktop download call; depends on M3 | At most three attempts, delayed between attempts, with a 45-second window for starting retries. Only Share remote read failures retry; each starts from byte zero. Final output is complete, failures leave no partial published file. Local errors, permission denial, protocol corruption and source drift stop. IDs, empty files, exports and non-Share backends preserve their contracts. The successful attempt supplies the conflict baseline. |
| M5: integration and delivery | Focused task fixtures, one `native/test-direct-open-task.py`, one manual Windows/Linux workflow, root graph, README/release docs; depends on M1–M4 | Real Direct reconnect plus daemon/agent streaming, authorization/identity, Room/shared read path, recovery lifecycle, conflict/save-back and stage cleanup checked in one remote pipeline. Reuse source/hash-bound library fixture or incremental host-library build. Commit/push milestones, evaluate the suite, then one existing complete-release dispatch. |

For M1–M4, no protocol capability, global timeout, Android background preference, sync path,
mount identity, mutation replay rule, or release procedure is changed. CI uses
isolated profiles and no GUI launch. A physical Android network/Doze transition
cannot be proven by loopback fixtures and must be reported as a validation limit.

The active batch now also includes [Drive duplicate conflict repair](DRIVE_SYNC_DUPLICATES_REPAIR.md).
The earlier Direct candidate `ea553ef2952fa72df2278207a92b79b1c8cc345b`
passed Windows/Linux in [run 36618661175](https://github.com/b1ue-man/smart-explorer/actions/runs/36618661175).
That result predates the final save-back revision guard and added sync work.
The same entrypoint now covers the complete candidate; its final acceptance and
the one terminal publication remain pending.
