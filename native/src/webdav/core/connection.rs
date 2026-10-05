//! Agent construction preserves distinct read, metadata and mutation contracts.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};

impl WebdavBackend {
    pub fn connect(cfg: WebdavConfig) -> io::Result<WebdavBackend> {
        let scheme = if cfg.https { "https" } else { "http" };
        let host = cfg.host.trim();
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        let base = format!("{scheme}://{host}:{}", cfg.port);
        let auth = if cfg.user.is_empty() {
            String::new()
        } else {
            format!(
                "Basic {}",
                STANDARD.encode(format!("{}:{}", cfg.user, cfg.password))
            )
        };
        let agent = transport_builder()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(IO_INACTIVITY_TIMEOUT)
            .timeout_write(IO_INACTIVITY_TIMEOUT)
            .max_idle_connections_per_host(IDLE_CONNECTIONS_PER_HOST)
            .build();
        let metadata_agent = transport_builder()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(IO_INACTIVITY_TIMEOUT)
            .timeout_write(IO_INACTIVITY_TIMEOUT)
            .redirects(0)
            .max_idle_connections_per_host(IDLE_CONNECTIONS_PER_HOST)
            .build();
        let mutation_agent = transport_builder()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(IO_INACTIVITY_TIMEOUT)
            .timeout_write(IO_INACTIVITY_TIMEOUT)
            .redirects(0)
            .max_idle_connections(0)
            .build();
        let write_agent = transport_builder()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(IO_INACTIVITY_TIMEOUT)
            .timeout_write(IO_INACTIVITY_TIMEOUT)
            .redirects(0)
            .max_idle_connections_per_host(IDLE_CONNECTIONS_PER_HOST)
            .build();
        let root = if cfg.root.trim().is_empty() {
            "/".to_string()
        } else {
            cfg.root.to_string()
        };
        let identity = format!("webdav:{base}:user={}:root={root}", cfg.user);
        let be = WebdavBackend {
            url: format!("webdav {}{}", base, root),
            base,
            root: root.clone(),
            auth,
            agent,
            metadata_agent,
            mutation_agent,
            write_agent,
            identity,
            hashes_observed: Arc::new(AtomicBool::new(false)),
            stage_times: Arc::new(Mutex::new(HashMap::new())),
        };
        // Validate credentials / reachability up front.
        be.propfind(&root, "0")?;
        Ok(be)
    }
}
