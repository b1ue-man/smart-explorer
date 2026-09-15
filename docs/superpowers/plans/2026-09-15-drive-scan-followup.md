# Drive root-scan follow-up — 2026-09-15

## Goal and evidence boundary

Investigate the user's remaining `Scan-Fehler: 1 gesamt, 1 Pfade im Protokoll`
with only `/` supplied. Identify the actual failure from the installed version
and full report; fix demonstrated defects in the affected listing/diagnostic
path. The supplied excerpt does not prove an HTTP, authentication, or name error.
The version and complete error details have been requested.

## Stage one: current code and approach

The starting source is `764af98` (0.5.157). The graph and source trace is
`GDriveBackend::list_dir` → `rscan::WalkState` → `drain_scan_channel` →
`App::error_log_text`. Listing failures retain `list_dir: <detail>` in
`FailedPaths`; ordinary listing failures do not also emit an app-wide error.
The report currently joins path and detail with a tab and has no version or
backend context. Its editable, fixed-row text area can obscure long reports.

A separate, proven compatibility defect is present in the newly strict Drive
listing and name-query parsing: both reject an omitted `files` field. Google's
official Python quickstart uses `results.get("files", [])`. Correct this
without treating malformed JSON, invalid field types, or incomplete searches
as successful empty results. Improve the report so a future copy contains an
unambiguous path/cause pair and the application version.

## First research pass

Primary sources checked on 2026-09-15:

- [Drive Python quickstart](https://developers.google.com/workspace/drive/api/quickstart/python):
  absent `files` is handled as an empty list.
- [files.list](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list):
  consume `nextPageToken` until exhausted and distinguish `incompleteSearch`.
- [Drive error handling](https://developers.google.com/workspace/drive/api/guides/handle-errors):
  HTTP status plus JSON message/reason determine the error; a root path alone
  does not determine the cause.
- [ProtoJSON presence and defaults](https://protobuf.dev/programming-guides/json/#presence-and-default-values):
  empty repeated fields may be omitted; null field values have unset semantics.
- The pinned egui 0.29.1 `widgets/text_edit/builder.rs` documents selectable
  read-only text through `&mut &str`; scrolling and the shared window limits
  already have established implementations in this repository.

## Stage two: final milestone plan

| Milestone | Affected boundary | Expected result in the single remote suite |
| --- | --- | --- |
| F1: valid empty Drive pages | New `gdrive/core/file_list.rs`; `backend.rs`, `resolution.rs`, `promotion_api.rs` | Omitted/null/empty `files` in a JSON object is empty. Continuation tokens are still followed. Empty root/child folders and absent-name lookups succeed. Invalid bodies/types, incomplete searches and repeated tokens remain errors; mutations do not follow untrustworthy absence results. |
| F2: complete readable error reports | Extract report rendering from `app/core/status_errors.rs` into `error_log_ui.rs` | Version, backend and root accompany diagnostics. Every scan item has separate path and cause lines. Empty details are explicitly identified. Long read-only reports scroll within the viewport, and Copy preserves the complete text. |
| F3: remaining reported failure | Installed version and full user report; the concrete affected module if evidence identifies another defect | Reproduce and correct the supplied cause before claiming that the user's live scan is repaired. If those details remain unavailable, report that boundary honestly. |

## Second research pass / integration decisions

- Accept absent/null fields only inside a successful JSON object. A null body,
  array body or wrongly typed `files`, `nextPageToken` or `incompleteSearch`
  remains an error. Empty/null continuation tokens mean no further page.
- Share the small page parser across folder listings, same-name resolution and
  promotion's named-object query, preventing different absence semantics at a
  write boundary. Promotion must request and check `incompleteSearch` too.
- Keep HTTP error message/reason intact through the real scan channel and GUI
  clipboard flow. Do not infer a login problem or reset credentials.
- Update the existing combined GUI/Drive task entrypoint only after the full
  implementation is ready. Use its source-bound incremental host-library
  binary and existing egui capture/HTTP fixtures, with follow-up selection for
  only these behaviors and directly affected name/mutation regressions.
- No local build, test or native execution. Commit coherent milestones, push
  the completed candidate, evaluate the one remote entrypoint, then use the
  established terminal release wrapper once after the complete batch is ready.

## Implementation checkpoint

F1 is implemented in `edcc3a5`; F2 in `de9d183`. Clearing the app log also clears
its current error so the next frame cannot immediately recreate the cleared
entry; retained scan-path diagnostics remain available. Only static Rust syntax
and text checks have run. The full supplied failure (F3) remains unconfirmed;
do not claim the user's scan repaired from the `/` excerpt. Remote acceptance
and the terminal release belong to the completed follow-up batch. Live open
status is tracked as G2 in `docs/TODO.md`.

The [desktop correction batch](2026-09-15-desktop-gui-correction.md) now includes
F1/F2 in the same existing remote entrypoint: real empty/intermediate pages,
strict invalid responses, mutation absence safety, and an HTTP failure through
the scan channel to the readable report and full clipboard output. F3 remains
a live evidence gap rather than an inferred authentication diagnosis.
