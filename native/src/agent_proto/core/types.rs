/// Bumped whenever the wire format OR the agent's behaviour changes; the client
/// re-uploads the agent on a mismatch.
pub const PROTO_VERSION: u32 = 9;

/// Existing error frames can request a metadata walk without changing wire
/// layouts. A hash-only stream cannot represent protected link boundaries.
pub const HASH_WALK_LINK_BOUNDARY: &str = "SE_HASH_WALK_LINK_BOUNDARY_V1";
/// Optional capability carried by the existing, display-only Hello version.
pub const HASH_WALK_SERVER_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+sync-links-v1");

pub fn has_link_aware_hash(version: &str) -> bool {
    version
        .split_whitespace()
        .next()
        .and_then(|text| text.split_once('+'))
        .is_some_and(|(_, labels)| labels.split('.').any(|label| label == "sync-links-v1"))
}

/// Payload chunk size for streamed byte transfers.
pub const CHUNK: usize = 256 * 1024;

/// Per-request frame backlog. At the maximum data-frame size this permits
/// about 8 MiB of pipelining while applying end-to-end streaming backpressure.
pub const TRANSFER_FRAME_BACKLOG: usize = 32;

/// Backend-neutral directory entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WireMeta {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mtime_ms: i64,
    pub content_md5: Option<String>,
}

/// One node of the size tree.
#[derive(Clone, Debug, PartialEq)]
pub struct WireNode {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub children: Vec<WireNode>,
}

/// One new file of a batch upload: final path, exact length and the client
/// nonce that names its private stage, so the stage of an ambiguous outcome
/// can be identified later.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchEntry {
    pub path: String,
    pub size: u64,
    pub nonce: u64,
}

/// One file of a batch download. `id` is forwarded to ID-addressed backends
/// behind the background service; a plain filesystem ignores it.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchItem {
    pub path: String,
    pub id: Option<String>,
    pub size: u64,
}

/// A server-side search request.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchSpec {
    pub query: String,
    pub glob: bool,
    pub min_size: u64,
    /// 0 = no upper bound.
    pub max_size: u64,
    /// 0 = unlimited.
    pub max_results: u64,
    /// Match directories too (else only files).
    pub want_dirs: bool,
}

/// One frame on the wire. Requests, responses and stream chunks all ride the
/// same enum so the channel is fully bidirectional.
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    Hello {
        proto: u32,
    },
    HelloOk {
        proto: u32,
        version: String,
    },
    ListDir(String),
    Dir(Vec<WireMeta>),
    Stat(String),
    Meta(WireMeta),
    /// Fallible existence probe. Only a genuine not-found result maps to
    /// `Exists(false)`; all other failures are returned as `Err`.
    TryExists(String),
    Exists(bool),
    WalkTree(String),
    Tree(WireNode),
    /// Read `len` bytes from `offset` (len 0 = to EOF) -> `Data`* `End`.
    Read {
        path: String,
        offset: u64,
        len: u64,
    },
    /// Begin writing `path`; client follows with `Data`* `End` -> `Ok`.
    Write(String),
    /// Begin writing a new file; final promotion fails if `path` already exists.
    WriteNew(String),
    /// A chunk of a byte stream.
    Data(Vec<u8>),
    Copy {
        src: String,
        dst: String,
    },
    Rename {
        src: String,
        dst: String,
    },
    /// Atomically rename `src` only when `dst` does not exist.
    RenameNoReplace {
        src: String,
        dst: String,
    },
    /// Commit a complete staged file using the backend's safe replace
    /// primitive. This must not degrade into an ordinary rename client-side.
    Promote {
        staged: String,
        destination: String,
    },
    /// Commit a complete staged file only if the destination remains absent.
    PromoteNoReplace {
        staged: String,
        destination: String,
    },
    Remove {
        path: String,
        recursive: bool,
    },
    Mkdir(String),
    /// Stream an entire subtree down.
    GetTree(String),
    /// Receive an entire subtree.
    PutTree(String),
    /// Header for one entry inside a Get/PutTree stream. `rel` must be a
    /// validated, non-empty path relative to the requested transfer root.
    TreeEntry {
        rel: String,
        is_dir: bool,
        size: u64,
        mtime_ms: i64,
    },
    Search {
        root: String,
        spec: SearchSpec,
    },
    Match {
        rel: String,
        is_dir: bool,
        size: u64,
        mtime_ms: i64,
    },
    WalkHashed {
        root: String,
        want_hash: bool,
    },
    HashEntry {
        rel: String,
        is_dir: bool,
        size: u64,
        mtime_ms: i64,
        md5: Option<String>,
    },
    Progress {
        done: u64,
        total: u64,
    },
    Ok,
    End,
    Err(String),
    Cancel,
    /// Flow control (`credit-v1`): the receiver of a request's stream allows
    /// its peer `bytes` more stream bytes. Request id 0 switches the whole
    /// connection to credit mode; only a client sends that.
    Credit {
        bytes: u64,
    },
    /// Batch upload header (`batch-v1`); per entry `Data`* then `ItemEnd`,
    /// finally `End`. Replies `ItemPublished`/`ItemFailed` per entry, `Ok`.
    BatchPut {
        entries: Vec<BatchEntry>,
    },
    /// Batch download; replies per item `ItemBegin` `Data`* `ItemEnd` or
    /// `ItemFailed`, finally `End`.
    BatchGet {
        items: Vec<BatchItem>,
    },
    ItemBegin {
        index: u32,
        size: u64,
    },
    /// End of one batch item's bytes; `error` = the bytes must be discarded
    /// (source changed or could not be read completely).
    ItemEnd {
        index: u32,
        error: Option<String>,
    },
    ItemPublished {
        index: u32,
        path: String,
    },
    ItemFailed {
        index: u32,
        message: String,
    },
    /// Server-side copy of `src` (expected length `size`) into the new,
    /// exclusively created private stage `stage` (`stage-v1`).
    CopyToStage {
        src: String,
        stage: String,
        size: u64,
    },
    /// Reply to `CopyToStage`: copied bytes, `None` = copy there by streaming.
    Copied(Option<u64>),
    /// Create one directory whose parent exists (`stage-v1`); `exclusive`
    /// fails with "already exists" when the name is taken.
    CreateDir {
        path: String,
        exclusive: bool,
    },
    /// Remove an unpublished private stage this client created (`stage-v1`).
    DiscardStage(String),
}
