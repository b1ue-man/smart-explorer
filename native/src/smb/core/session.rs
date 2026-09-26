//! One SMB session per backend. A generation is one authenticated
//! connection (TCP, NEGOTIATE, SESSION_SETUP) with a lazily connected tree
//! per share. A proven dead connection retires its generation; the next
//! call dials a new one and its trees reconnect on first use. There is no
//! DFS and no smb2 auto-reconnect: the connect stages are driven here so
//! each failure keeps its own meaning (network, SMB1-only server, sign-in).
use super::errors;
use super::url::{server_addr, split_domain_user};
use smb2::client::{Cipher, Connection};
use smb2::{Session, Tree};
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::runtime::Runtime;

/// Whole TCP connect budget, name resolution included.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Connection settings. `user` may be `DOMAIN\user`; `root` is the start
/// path `/<share>/<path>` whose share is connected (and so verified) first.
#[derive(Clone)]
pub struct SmbConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub root: String,
}

pub(super) struct Generation {
    trees: Mutex<HashMap<String, Arc<Tree>>>,
    dead: AtomicBool,
    session: Session,
    conn: Connection,
}

impl Generation {
    /// A cheap clone sharing the session (smb2 multiplexes clones).
    pub(super) fn connection(&self) -> Connection {
        self.conn.clone()
    }

    fn alive(&self) -> bool {
        !self.dead.load(Ordering::Acquire) && !self.conn.is_disconnected()
    }

    /// Retires the generation after a lost connection (true: a replay on a
    /// new one is safe) or a timeout (retired, but nothing is replayed).
    pub(super) fn note(&self, error: &smb2::Error) -> bool {
        let dead = errors::is_dead(error);
        if dead || errors::is_suspect(error) {
            self.dead.store(true, Ordering::Release);
        }
        dead
    }
}

pub(super) struct SmbSession {
    config: SmbConfig,
    current: Mutex<Option<Arc<Generation>>>,
    // Declared last: dropped after every connection it drives.
    rt: Arc<Runtime>,
}

fn lock<T>(mutex: &Mutex<T>) -> io::Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| io::Error::other("SMB-Sitzungszustand ist inkonsistent"))
}

impl SmbSession {
    /// Connects and tree-connects `share`, so a wrong share name fails here.
    pub(super) fn connect(config: SmbConfig, share: &str) -> io::Result<Arc<Self>> {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("smb")
                .enable_all()
                .build()?,
        );
        let generation = Arc::new(connect_generation(&rt, &config)?);
        let session = Arc::new(Self {
            config,
            current: Mutex::new(Some(generation.clone())),
            rt,
        });
        session.tree(&generation, share)?;
        Ok(session)
    }

    pub(super) fn runtime(&self) -> Arc<Runtime> {
        self.rt.clone()
    }

    pub(super) fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.rt.block_on(future)
    }

    /// The live generation; a retired one is replaced first. Reconnects are
    /// serialized, so concurrent callers share one new session.
    pub(super) fn current(&self) -> io::Result<Arc<Generation>> {
        let mut current = lock(&self.current)?;
        if let Some(generation) = current.as_ref().filter(|generation| generation.alive()) {
            return Ok(generation.clone());
        }
        *current = None;
        let replacement = Arc::new(connect_generation(&self.rt, &self.config)?);
        *current = Some(replacement.clone());
        Ok(replacement)
    }

    /// The tree of `share` on `generation`, connected on first use.
    pub(super) fn tree(&self, generation: &Generation, share: &str) -> io::Result<Arc<Tree>> {
        let mut trees = lock(&generation.trees)?;
        if let Some(tree) = trees.get(share) {
            return Ok(tree.clone());
        }
        let mut conn = generation.connection();
        let mut tree = self
            .rt
            .block_on(Tree::connect(&mut conn, share))
            .map_err(|error| {
                generation.note(&error);
                errors::share_failed(share, error)
            })?;
        // As `SmbClient::connect_share` sets it: smb2 derives the host of a
        // DFS-capable share's paths from `host:port`.
        tree.server = server_addr(&self.config.host, self.config.port);
        activate_share_encryption(&mut conn, &generation.session, &tree);
        let tree = Arc::new(tree);
        trees.insert(share.to_string(), tree.clone());
        Ok(tree)
    }

    /// The live generation and the tree of `share` on it. A tree connect that
    /// finds the connection gone is repeated once on a new session (nothing
    /// on the share has changed yet).
    pub(super) fn target(&self, share: &str) -> io::Result<(Arc<Generation>, Arc<Tree>)> {
        let generation = self.current()?;
        match self.tree(&generation, share) {
            Ok(tree) => Ok((generation, tree)),
            Err(error) if errors::is_connection_loss(error.kind()) => {
                let generation = self.current()?;
                let tree = self.tree(&generation, share)?;
                Ok((generation, tree))
            }
            Err(error) => Err(error),
        }
    }

    /// Runs an idempotent request; after a proven connection loss it is
    /// replayed once on a new session.
    pub(super) fn read<T, F, Fut>(
        &self,
        share: &str,
        action: &str,
        path: &str,
        operation: F,
    ) -> io::Result<T>
    where
        F: Fn(Connection, Arc<Tree>) -> Fut,
        Fut: Future<Output = smb2::Result<T>>,
    {
        let (generation, tree) = self.target(share)?;
        let error = match self.rt.block_on(operation(generation.connection(), tree)) {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        if !generation.note(&error) {
            return Err(errors::map(error, action, path));
        }
        let (generation, tree) = self.target(share)?;
        self.rt
            .block_on(operation(generation.connection(), tree))
            .map_err(|error| {
                generation.note(&error);
                errors::map(error, action, path)
            })
    }

    /// Runs a request that changes the share; it is never replayed (it may
    /// have taken effect before the connection was lost).
    pub(super) fn write<T, F, Fut>(
        &self,
        share: &str,
        action: &str,
        path: &str,
        operation: F,
    ) -> io::Result<T>
    where
        F: FnOnce(Connection, Arc<Tree>) -> Fut,
        Fut: Future<Output = smb2::Result<T>>,
    {
        let (generation, tree) = self.target(share)?;
        self.rt
            .block_on(operation(generation.connection(), tree))
            .map_err(|error| {
                generation.note(&error);
                errors::map(error, action, path)
            })
    }
}

fn connect_generation(rt: &Runtime, config: &SmbConfig) -> io::Result<Generation> {
    let addr = server_addr(&config.host, config.port);
    let (domain, user) = split_domain_user(&config.user);
    rt.block_on(async {
        let mut conn = Connection::connect(&addr, CONNECT_TIMEOUT)
            .await
            .map_err(|error| errors::connect_failed(&config.host, config.port, error))?;
        conn.negotiate().await.map_err(errors::negotiate_failed)?;
        let session = Session::setup(&mut conn, &user, &config.password, &domain)
            .await
            .map_err(errors::session_failed)?;
        Ok::<Generation, io::Error>(Generation {
            trees: Mutex::new(HashMap::new()),
            dead: AtomicBool::new(false),
            session,
            conn,
        })
    })
}

/// A share flagged ENCRYPT_DATA needs encryption on the connection, keyed by
/// the session (the same step `SmbClient::connect_share` performs; a session
/// that already encrypts everything needs nothing more).
fn activate_share_encryption(conn: &mut Connection, session: &Session, tree: &Tree) {
    if !tree.encrypt_data || conn.should_encrypt() {
        return;
    }
    let (Some(encryption), Some(decryption)) = (&session.encryption_key, &session.decryption_key)
    else {
        return;
    };
    let cipher = conn
        .params()
        .and_then(|params| params.cipher)
        .unwrap_or(Cipher::Aes128Ccm);
    conn.activate_encryption(encryption.clone(), decryption.clone(), cipher);
}
