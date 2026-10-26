# Smart Explorer GL error propagation patch

Baseline: the published `egui_glow` 0.29.1 crate, SHA-256
`0e39bccc683cd43adab530d8f21a13eb91e80de10bcc38c3f1c16601b6f62b26`.
The archive was verified against the existing Cargo.lock before copying its
source. Upstream `LICENSE-MIT` and `LICENSE-APACHE` were retrieved at tag 0.29.1
and are preserved in this directory. No local compilation took place.

The patch supports the owned screen/login renderer's operation result. Upstream
GL diagnostic macros consume error flags without returning upload/draw failure;
a new presentation receipt must not be accepted after that failure. The exact
implementation and static author evidence are maintained with the F3 follow-up
in `docs/plaene/2026-10-24-batches/renderer-speicher.md`. The source candidate now propagates those errors through the resident and
retirement decision to the presentation receipt. Its critical static review is
in progress; a successful runtime check is not claimed.

The patch adds no native runtime library. The task-suite binary fingerprint
includes this entire vendor directory. The upstream version stays pinned and
its existing rendering path remains the baseline. Remove this patch when an
upstream version supplies equivalent fallible upload/draw results, including
errors consumed by its internal diagnostics, and the shared screen/login
presentation and resource-retirement contracts have been checked against that
version. A version update must not silently discard this error contract.

Primary source checked 2026-10-26:
https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/egui_glow/src/lib.rs

The remote suite targets only this library's error-ledger fixtures through the
existing native lock and target cache. The vendored manifest excludes the
upstream standalone window example and its two example-only dev dependencies;
the original manifest and example source remain as upstream reference. This
keeps the library a patched dependency without introducing unrelated optional
workspace resolution. Cargo's package-selection guard rejects nonmember test
packages only when they declare dev dependencies; the ledger fixtures use only
existing library dependencies. The suite selects `-p egui_glow --lib --no-run`
and then its named fixture prefix, never all targets.

Primary package-selection source checked 2026-10-26:
https://raw.githubusercontent.com/rust-lang/cargo/master/src/ops/cargo_compile/mod.rs
