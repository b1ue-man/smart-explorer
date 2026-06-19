/// Bumped whenever the wire format OR the agent's behaviour changes; the client
/// re-uploads the agent on a mismatch.
pub const PROTO_VERSION: u32 = 4;

/// Payload chunk size for streamed byte transfers.
pub const CHUNK: usize = 256 * 1024;

/// Backend-neutral directory entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WireMeta {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mtime_ms: i64,
}

/// One node of the size tree.
#[derive(Clone, Debug, PartialEq)]
pub struct WireNode {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub children: Vec<WireNode>,
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
    Remove {
        path: String,
        recursive: bool,
    },
    Mkdir(String),
    /// Stream an entire subtree down.
    GetTree(String),
    /// Receive an entire subtree.
    PutTree(String),
    /// Header for one entry inside a Get/PutTree stream.
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
}
