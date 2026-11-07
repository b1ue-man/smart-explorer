//! WP2 task tests of „Remote Desktop“ as one action (design 2, 10.2 V4–V6,
//! assumptions A4–A6). Prefix `einrichtung_task_`. Tests marked `ignore`
//! need a prepared runner (packages, display, a paired host) and run only
//! through the remote task-suite entrypoint with `--ignored`.
use crate::rdp::core::{
    AuthenticatedProbe, CertificateFingerprint, CertificatePolicy, CertificateSha1,
    ClientCapability, ClientKind, HostKind, HostProbe, HostService, SessionMode, StartRequest,
    RDP_PROBE_VERSION_V2,
};
use std::sync::mpsc;
use std::time::Duration;

fn service(id: &str, kind: HostKind, mode: SessionMode, port: u16) -> HostService {
    HostService {
        id: id.into(),
        kind,
        version: None,
        session_mode: mode,
        port: Some(port),
        running: true,
        tls: Some(true),
        nla: Some(true),
        certificate_sha256: Some(CertificateFingerprint::parse(&"ab".repeat(32)).unwrap()),
        certificate_source: Some("host-listener-certificate".into()),
        remote_apps: Vec::new(),
        evidence: vec!["listener owned by service".into()],
        unavailable_reason: None,
        certificate_sha1: None,
        managed_credentials: false,
        listener_pid: None,
    }
}

fn probe(services: Vec<HostService>) -> AuthenticatedProbe {
    let report = HostProbe {
        version: RDP_PROBE_VERSION_V2,
        platform: "linux".into(),
        services,
        unavailable_reason: None,
        diagnostics: Vec::new(),
    };
    AuthenticatedProbe::from_authenticated_peer("host-node".into(), report).unwrap()
}

fn client(id: &str, kind: ClientKind, policy: CertificatePolicy) -> ClientCapability {
    ClientCapability {
        id: id.into(),
        kind,
        version: None,
        certificate_policy: policy,
        remote_app: true,
        available: true,
        detail: String::new(),
    }
}

/// Design 2.3: the shared console wins, then the extra display, then a
/// headless session, then separate Xorg/Xvnc sessions.
#[test]
fn einrichtung_task_service_rank_prefers_the_visible_console() {
    let gnome = service("gnome", HostKind::Gnome, SessionMode::SharedConsole, 3389);
    let xrdp = service("xrdp", HostKind::Xrdp, SessionMode::SeparateXorg, 3390);
    let both = probe(vec![xrdp.clone(), gnome.clone()]);
    assert_eq!(both.choose_service(None).unwrap().id, "gnome");
    let mut stopped = gnome.clone();
    stopped.running = false;
    let fallback = probe(vec![stopped, xrdp.clone()]);
    assert_eq!(fallback.choose_service(None).unwrap().id, "xrdp");
    // `--service` stays an override.
    assert_eq!(both.choose_service(Some("xrdp")).unwrap().id, "xrdp");
    let mut down = xrdp;
    down.running = false;
    let none = probe(vec![down]);
    let error = none.choose_service(None).unwrap_err();
    assert!(error.contains("Kein startbarer"), "{error}");
}

/// Design 2.6/2.9: the request is derived; old caller fields are accepted
/// and ignored; the tunnel has no fixed lifetime.
#[test]
fn einrichtung_task_start_request_is_derived_without_forms() {
    let xrdp = service("xrdp", HostKind::Xrdp, SessionMode::SeparateXorg, 3389);
    let pin = CertificatePolicy::PeerPinOrNativeTrust;
    let freerdp = client("sdl-freerdp3", ClientKind::FreeRdp3, pin);
    let request = StartRequest::derive(&xrdp, &freerdp, None);
    assert_eq!(
        request.certificate_policy,
        CertificatePolicy::PeerPinOrNativeTrust
    );
    let probe = probe(vec![xrdp.clone()]);
    request.validate(&probe).unwrap();
    let tunnel = request.tunnel_request(&xrdp).unwrap();
    assert_eq!(tunnel.lifetime_secs, 0);
    assert!(tunnel.acknowledge_local_access);
    assert_eq!(tunnel.local_port, 0);
    let old: StartRequest = serde_json::from_str(
        r#"{"service_id":"xrdp","client_id":"xfreerdp3",
            "certificate_policy":"peer_pin_or_native_trust","remote_app":null,
            "acknowledge_local_access":true,"acknowledge_session_effect":true,
            "acknowledge_certificate_dialog":false,"lifetime_secs":900}"#,
    )
    .unwrap();
    assert_eq!(old.service_id, "xrdp");
    let mut exact = request;
    exact.certificate_policy = CertificatePolicy::ExactPeerPin;
    assert!(exact.validate(&probe).is_err());
}

/// Design 2.7.2: mstsc needs the probe v2 thumbprint; an old host is named.
#[test]
fn einrichtung_task_mstsc_requires_the_probe_v2_thumbprint() {
    let mut windows = service(
        "termservice",
        HostKind::WindowsWorkstation,
        SessionMode::WindowsConsoleLocked,
        3389,
    );
    let mstsc = client("mstsc", ClientKind::Mstsc, CertificatePolicy::NativeDialog);
    let error = StartRequest::require_client_evidence(&windows, &mstsc).unwrap_err();
    assert_eq!(error, crate::rdp::OLD_HOST);
    windows.certificate_sha1 = Some(CertificateSha1::parse(&"CD".repeat(20)).unwrap());
    StartRequest::require_client_evidence(&windows, &mstsc).unwrap();
    let pin = CertificatePolicy::PeerPinOrNativeTrust;
    let freerdp = client("xfreerdp3", ClientKind::FreeRdp3, pin);
    windows.certificate_sha1 = None;
    StartRequest::require_client_evidence(&windows, &freerdp).unwrap();
}

// Compiles only when `T` does NOT implement `Clone` (two applicable impls
// would make the inference ambiguous).
trait AmbiguousIfClone<A> {
    fn check() {}
}
impl<T> AmbiguousIfClone<()> for T {}
impl<T: Clone> AmbiguousIfClone<u8> for T {}

/// V6: one probe, one start. `RdpSession::start` takes the probe by value
/// and the probe cannot be cloned, so a second start without a new probe
/// does not compile. No age limit applies any more.
#[test]
fn einrichtung_task_probe_is_consumed_by_one_start() {
    <AuthenticatedProbe as AmbiguousIfClone<_>>::check();
    fn by_value(_: AuthenticatedProbe) {}
    let probe = probe(vec![service(
        "xrdp",
        HostKind::Xrdp,
        SessionMode::SeparateXorg,
        3389,
    )]);
    assert!(probe.service("xrdp").is_ok());
    by_value(probe);
}

fn sleeper() -> std::process::Child {
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("powershell");
        command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 120"]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = std::process::Command::new("sleep");
        command.arg("120");
        command
    };
    command.spawn().expect("test helper process starts")
}

/// Design 2.6: the daemon's handle of the bound client ends the tunnel when
/// the client ends; a reused PID (other start time) is refused.
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn einrichtung_task_process_exit_watch_fires_and_refuses_reused_pids() {
    let mut child = sleeper();
    let pid = child.id();
    let started = crate::share::os_sockets::process_identity(pid)
        .unwrap()
        .started;
    let wrong = crate::rdp::watch_process_exit(pid, started.wrapping_add(1), Box::new(|| {}));
    assert!(wrong.is_err(), "another start time must be refused");
    let (sender, receiver) = mpsc::channel();
    crate::rdp::watch_process_exit(
        pid,
        started,
        Box::new(move || {
            let _ = sender.send(());
        }),
    )
    .unwrap();
    assert!(receiver.recv_timeout(Duration::from_millis(500)).is_err());
    child.kill().unwrap();
    child.wait().unwrap();
    receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("exit of the bound process is observed");
}

/// A6 storage form (design 2.7.2): `CertHash` is written as 20 bytes under
/// the device address and under `<address>:<port>`.
#[cfg(windows)]
#[test]
fn einrichtung_task_cert_hash_is_stored_for_the_device_address() {
    use crate::rdp::os::platform::clients::trust;
    let listener: std::net::SocketAddr = "127.201.7.9:51234".parse().unwrap();
    let sha1 = CertificateSha1::parse(&"0A1B".repeat(10)).unwrap();
    trust::write_cert_hash(listener, &sha1).unwrap();
    for name in trust::server_names(listener) {
        assert_eq!(
            trust::read_cert_hash(&name).as_deref(),
            Some(&sha1.bytes()[..])
        );
        let path = format!(r"Software\Microsoft\Terminal Server Client\Servers\{name}");
        let _ = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey(&path);
    }
}

/// Design 2.8: se-managed credentials are a session credential
/// `TERMSRV/<address>` that ends with the RDP session.
#[cfg(windows)]
#[test]
fn einrichtung_task_managed_credential_lives_with_the_session() {
    let listener: std::net::SocketAddr = "127.202.8.10:51235".parse().unwrap();
    let mut gnome = service("gnome", HostKind::Gnome, SessionMode::SharedConsole, 3389);
    gnome.certificate_sha1 = Some(CertificateSha1::parse(&"11".repeat(20)).unwrap());
    let credentials = crate::rdp::Credentials {
        username: "se-0011aabb".into(),
        domain: None,
        password: crate::share::apps_wire::ManagedSecret::new("task-secret".into()),
    };
    let listed = || {
        let output = std::process::Command::new("cmdkey")
            .arg("/list:TERMSRV/127.202.8.10")
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).contains("se-0011aabb")
    };
    let guard =
        crate::rdp::os::shared::clients::prepare_trust(listener, &gnome, Some(&credentials))
            .unwrap();
    assert!(listed(), "session credential is visible to mstsc");
    drop(guard);
    assert!(!listed(), "session credential is deleted with the session");
    for name in crate::rdp::os::platform::clients::trust::server_names(listener) {
        let path = format!(r"Software\Microsoft\Terminal Server Client\Servers\{name}");
        let _ = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey(&path);
    }
}

/// A4: Debian/Ubuntu ship the SDL client as `sdl-freerdp3` (package
/// `freerdp3-sdl`); it is the first available client (design 2.4).
#[cfg(target_os = "linux")]
#[test]
#[ignore = "remote suite: needs freerdp3-sdl installed and a DISPLAY (Xvfb); Suite speed-setup-task"]
fn einrichtung_task_sdl_freerdp_is_the_first_client() {
    let clients = crate::rdp::local_clients();
    let first = clients.first().expect("a FreeRDP client is installed");
    assert_eq!(first.id, "sdl-freerdp3", "{clients:?}");
    assert!(first.available, "{}", first.detail);
    let chosen = crate::rdp::choose_client(None).unwrap();
    assert_eq!(chosen.id, "sdl-freerdp3");
}
