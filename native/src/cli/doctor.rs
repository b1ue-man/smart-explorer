use clap::Args;
use std::io::Read;
use std::path::Path;

const MAX_DIAGNOSTIC_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Args)]
pub(super) struct DoctorArgs {
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

struct Health {
    state: &'static str,
    detail: String,
}

impl Health {
    fn ok(detail: impl Into<String>) -> Self {
        Self {
            state: "ok",
            detail: detail.into(),
        }
    }

    fn absent(detail: impl Into<String>) -> Self {
        Self {
            state: "not_configured",
            detail: detail.into(),
        }
    }

    fn error(detail: impl Into<String>) -> Self {
        Self {
            state: "error",
            detail: detail.into(),
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({ "state": self.state, "detail": self.detail })
    }
}

pub(super) fn run(args: DoctorArgs) -> Result<i32, String> {
    let app_data = crate::support_dirs::app_data_dir();
    let executable = std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| format!("current executable: {error}"))?;
    let connections = connection_health(&app_data);
    let profiles = profile_health();
    let identity = identity_health(&app_data);
    let server = share_server_health(&app_data);
    let daemon_running = crate::daemon::is_running();
    let heartbeat_age = crate::daemon::last_heartbeat_age();
    let credential_backend = crate::creds::secret_store_description();
    let credential_health = match crate::creds::probe_secret_store() {
        Ok(()) => Health::ok("credential backend is readable"),
        Err(error) => Health::error(error),
    };
    let exit_code = if [
        &credential_health,
        &connections,
        &profiles,
        &identity,
        &server,
    ]
    .iter()
    .any(|health| health.state == "error")
    {
        1
    } else {
        0
    };

    if args.json {
        let value = serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "executable": executable,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "app_data": app_data,
            "credential_backend": credential_backend,
            "credential_store": credential_health.json(),
            "connections": connections.json(),
            "share_profiles": profiles.json(),
            "share_identity": identity.json(),
            "share_server": server.json(),
            "daemon": {
                "running": daemon_running,
                "heartbeat_age_seconds": heartbeat_age,
            },
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
        return Ok(exit_code);
    }

    println!("version\t{}", env!("CARGO_PKG_VERSION"));
    println!("executable\t{executable}");
    println!(
        "platform\t{}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("app_data\t{}", app_data.display());
    println!("credential_backend\t{credential_backend}");
    print_health("credential_store", &credential_health);
    print_health("connections", &connections);
    print_health("share_profiles", &profiles);
    print_health("share_identity", &identity);
    print_health("share_server", &server);
    println!("daemon_running\t{daemon_running}");
    println!(
        "daemon_heartbeat_age_seconds\t{}",
        heartbeat_age
            .map(|age| age.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    Ok(exit_code)
}

fn print_health(label: &str, health: &Health) {
    println!("{label}\t{}\t{}", health.state, health.detail);
}

fn connection_health(app_data: &Path) -> Health {
    let path = app_data.join("connections.txt");
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Health::absent("no saved connection store")
        }
        Err(error) => return Health::error(error.to_string()),
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Health::error("connection metadata is not a regular file")
        }
        Ok(_) => {}
    }
    match crate::creds::load_connections_checked() {
        Ok(connections) => Health::ok(format!("{} saved connection(s)", connections.len())),
        Err(error) => Health::error(error),
    }
}

fn profile_health() -> Health {
    match crate::share::ShareProfiles::load_checked(None) {
        Ok(profiles) => Health::ok(format!(
            "{} peer(s), {} room(s)",
            profiles.direct_contacts.len(),
            profiles.rooms.len()
        )),
        Err(error) => Health::error(error),
    }
}

fn identity_health(app_data: &Path) -> Health {
    let path = app_data.join("share_identity.json");
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Health::absent("share identity has not been created")
        }
        Err(error) => Health::error(error.to_string()),
        Ok(metadata) if !metadata.file_type().is_file() => {
            Health::error("share identity is not a regular file")
        }
        Ok(metadata) if metadata.len() > MAX_DIAGNOSTIC_FILE_BYTES => {
            Health::error("share identity exceeds the diagnostic byte limit")
        }
        Ok(_) => match crate::share::ShareIdentity::load_or_create(default_device_name()) {
            Ok(_) => Health::ok("identity metadata and secrets are readable"),
            Err(error) => Health::error(error),
        },
    }
}

fn share_server_health(app_data: &Path) -> Health {
    let path = app_data.join("share_server.txt");
    match read_regular_bounded(&path) {
        Ok(None) => Health::absent("no share server configured"),
        Ok(Some(raw)) => {
            let server = raw.trim().to_string();
            if server.is_empty() {
                Health::absent("share server is empty")
            } else {
                match super::share::validate_server(&server) {
                    Ok(server) => Health::ok(format!("share server configured: {server}")),
                    Err(error) => Health::error(error),
                }
            }
        }
        Err(error) => Health::error(error),
    }
}

fn read_regular_bounded(path: &Path) -> Result<Option<String>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.file_type().is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if metadata.len() > MAX_DIAGNOSTIC_FILE_BYTES {
        return Err(format!(
            "{} exceeds the diagnostic byte limit",
            path.display()
        ));
    }
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(MAX_DIAGNOSTIC_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_DIAGNOSTIC_FILE_BYTES {
        return Err(format!(
            "{} exceeds the diagnostic byte limit",
            path.display()
        ));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("{} is not valid UTF-8", path.display()))
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Smart Explorer CLI".to_string())
}
