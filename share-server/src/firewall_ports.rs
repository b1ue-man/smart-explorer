//! se-share-server admits its own listening ports in the host firewall at
//! start (user orders 2026-11-25: „update, installation, NUTZUNG richten das
//! ein“, „ELEVATION FEHLT? ERFRAGEN!“): on Linux in ufw, firewalld, nftables
//! or legacy iptables while one of them filters input, on Windows with
//! program-bound rules of this program. Root (Linux) or an administrator
//! (Windows) changes the rules directly; otherwise Linux asks through sudo (in
//! a terminal; `sudo -n` without one, which works where sudo needs no
//! password) and Windows through the UAC prompt in a console. Every outcome
//! goes to standard error with the exact ports, so a service log names a
//! port that stayed closed.
use std::net::SocketAddr;
use std::process::{Command, Output, Stdio};

/// The comment that marks the server's own nftables and iptables rules.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) const MARK: &str = "se-share-server";
/// QUIC address discovery listens here unless `SE_IROH_QAD_BIND` says else
/// (`relay_qad::plan`).
const QAD_PORT: u16 = 7842;
/// Default ports of the address services (`tcp_reflect.rs`, `stun.rs`).
const TCP_REFLECT_PORT: u16 = 51823;
const STUN_PORT: u16 = 3478;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Proto {
    Tcp,
    Udp,
}

impl Proto {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

/// One listening socket of the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Listener {
    pub(crate) bind: SocketAddr,
    pub(crate) proto: Proto,
    pub(crate) purpose: &'static str,
}

impl Listener {
    pub(crate) fn tcp(bind: SocketAddr, purpose: &'static str) -> Self {
        Self {
            bind,
            proto: Proto::Tcp,
            purpose,
        }
    }

    pub(crate) fn udp(bind: SocketAddr, purpose: &'static str) -> Self {
        Self {
            bind,
            proto: Proto::Udp,
            purpose,
        }
    }

    /// `51820/tcp`, the form of ufw and firewalld.
    pub(crate) fn label(&self) -> String {
        format!("{}/{}", self.bind.port(), self.proto.name())
    }
}

/// The QAD listener of `relay_qad::plan`: none without TLS or when
/// `disable` is `1`/`true`; `bind` when given; else UDP 7842 at the relay IP.
pub(crate) fn qad_bind(
    relay: SocketAddr,
    tls: bool,
    disable: Option<&str>,
    bind: Option<&str>,
) -> Option<SocketAddr> {
    let disabled = disable.is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
    if disabled || !tls {
        return None;
    }
    match bind.map(str::trim).filter(|bind| !bind.is_empty()) {
        Some(bind) => bind.parse().ok(),
        None => Some(SocketAddr::new(relay.ip(), QAD_PORT)),
    }
}

/// The TCP address reflector and the STUN responder, as
/// `address_services::plan` reads `SE_SHARE_TCP_REFLECT_BIND` and
/// `SE_SHARE_STUN_BIND`: absent or empty, the signaling IP with the default
/// port; `off` or an invalid value, none; otherwise that `IP:PORT`.
pub(crate) fn service_bind(
    value: Option<&str>,
    signal: SocketAddr,
    default_port: u16,
) -> Option<SocketAddr> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Some(SocketAddr::new(signal.ip(), default_port)),
        Some(value) if value.eq_ignore_ascii_case("off") => None,
        Some(value) => value.parse().ok(),
    }
}

/// The listeners the internet must reach: signaling, the relay, QAD (with
/// TLS and a relay), the TCP reflector and the STUN responder, each from the
/// same settings the server starts them with. A listener on loopback stays
/// out; each port and protocol once.
pub(crate) fn server_ports(
    signal: SocketAddr,
    relay: Option<SocketAddr>,
    tls: bool,
) -> Vec<Listener> {
    let env = |name: &str| std::env::var(name).ok();
    let qad = relay.and_then(|relay| {
        qad_bind(
            relay,
            tls,
            env("SE_IROH_QAD_DISABLE").as_deref(),
            env("SE_IROH_QAD_BIND").as_deref(),
        )
    });
    let services: Vec<Listener> = [
        service_bind(
            env("SE_SHARE_TCP_REFLECT_BIND").as_deref(),
            signal,
            TCP_REFLECT_PORT,
        )
        .map(|bind| Listener::tcp(bind, "TCP-Adressreflektor")),
        service_bind(env("SE_SHARE_STUN_BIND").as_deref(), signal, STUN_PORT)
            .map(|bind| Listener::udp(bind, "STUN")),
    ]
    .into_iter()
    .flatten()
    .collect();
    collect(signal, relay, qad, &services)
}

pub(crate) fn collect(
    signal: SocketAddr,
    relay: Option<SocketAddr>,
    qad: Option<SocketAddr>,
    extra: &[Listener],
) -> Vec<Listener> {
    let mut ports: Vec<Listener> = Vec::new();
    let listeners = [Some(Listener::tcp(signal, "Signalisierung"))]
        .into_iter()
        .chain([relay.map(|bind| Listener::tcp(bind, "Iroh-Relay"))])
        .chain([qad.map(|bind| Listener::udp(bind, "QUIC-Adresserkennung (QAD)"))])
        .flatten()
        .chain(extra.iter().copied());
    for listener in listeners {
        let reachable = !listener.bind.ip().is_loopback();
        let known = ports
            .iter()
            .any(|port| port.bind.port() == listener.bind.port() && port.proto == listener.proto);
        if reachable && !known {
            ports.push(listener);
        }
    }
    ports
}

fn labels(ports: &[Listener]) -> String {
    let labels: Vec<String> = ports.iter().map(Listener::label).collect();
    labels.join(", ")
}

/// Why the platform left the ports closed.
pub(crate) struct Refusal {
    /// Neither root nor a usable sudo (Linux); no administrator or a declined
    /// UAC prompt (Windows).
    pub(crate) rights: bool,
    pub(crate) reason: String,
}

impl Refusal {
    #[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
    pub(crate) fn rights(reason: String) -> Self {
        Self {
            rights: true,
            reason,
        }
    }
}

impl From<String> for Refusal {
    fn from(reason: String) -> Self {
        Self {
            rights: false,
            reason,
        }
    }
}

/// The outcome of `admit`, after its log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Admission {
    /// Admitted, or nothing to admit.
    Done,
    /// Refused for lack of rights (`Refusal::rights`).
    NoRights,
    /// A firewall command failed.
    Failed,
}

/// Sets up the firewall for `ports` and logs the outcome; never ends the
/// server (a closed port is reported, the server still serves the open ones).
pub(crate) fn admit(ports: &[Listener]) -> Admission {
    if ports.is_empty() {
        return Admission::Done;
    }
    let (line, admission) = match platform::admit(ports) {
        Ok(status) => (status, Admission::Done),
        Err(refusal) => (
            format!("{}; nicht freigegeben: {}", refusal.reason, labels(ports)),
            if refusal.rights {
                Admission::NoRights
            } else {
                Admission::Failed
            },
        ),
    };
    eprintln!("se-share-server: firewall: {line}");
    admission
}

// The Linux privilege path: pure, so its tests also run on Windows.
/// How a firewall command runs: directly as root, else through sudo.
#[cfg_attr(windows, allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Privilege {
    Root,
    /// sudo asks for the password in this terminal.
    SudoAsk,
    /// `sudo -n`: no question; works where sudo needs no password.
    SudoQuiet,
}

impl Privilege {
    pub(crate) fn argv(self, tool: &str, args: &[String]) -> Vec<String> {
        let prefix: &[&str] = match self {
            Self::Root => &[],
            Self::SudoAsk => &["sudo"],
            Self::SudoQuiet => &["sudo", "-n"],
        };
        prefix
            .iter()
            .map(|word| word.to_string())
            .chain([tool.to_string()])
            .chain(args.iter().cloned())
            .collect()
    }
}

/// The effective user ID of `/proc/self/status` (`Uid:` real, effective, …).
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn effective_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|ids| ids.split_whitespace().nth(1))
        .and_then(|id| id.parse().ok())
}

#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn run(argv: &[String]) -> std::io::Result<Output> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| std::io::Error::other("leerer Befehl"))?;
    let mut command = Command::new(program);
    command.args(args).env("LC_ALL", "C");
    // sudo may ask in this terminal; every other call reads nothing.
    if argv.first().map(String::as_str) != Some("sudo")
        || argv.get(1).map(String::as_str) == Some("-n")
    {
        command.stdin(Stdio::null());
    }
    command.output()
}

#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub(crate) fn last_line(output: &Output) -> String {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("ohne Meldung")
        .chars()
        .take(300)
        .collect()
}

// The Windows rules: pure, so their tests also run on the Linux host.
/// `se-share-server TCP` or `se-share-server UDP`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn rule_name(proto: Proto) -> String {
    format!("se-share-server {}", proto.name().to_uppercase())
}

/// A path without the `\\?\` prefix: the firewall compares the path the
/// process runs from.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn plain(path: &str) -> String {
    match path.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => path.strip_prefix(r"\\?\").unwrap_or(path).to_string(),
    }
}

/// `netsh` arguments: this program's rule of each protocol deleted and added
/// again (another program's rule of that name stays).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn netsh_steps(program: &str, protos: &[Proto]) -> Vec<Vec<String>> {
    let mut steps = Vec::new();
    for proto in protos {
        let name = format!("name={}", rule_name(*proto));
        let program = format!("program={program}");
        let base = ["advfirewall", "firewall"].map(String::from);
        steps.push(
            base.iter()
                .cloned()
                .chain([
                    "delete".into(),
                    "rule".into(),
                    name.clone(),
                    program.clone(),
                ])
                .collect(),
        );
        steps.push(
            base.iter()
                .cloned()
                .chain([
                    "add".into(),
                    "rule".into(),
                    name,
                    "dir=in".into(),
                    "action=allow".into(),
                    program,
                    "enable=yes".into(),
                    "profile=any".into(),
                    format!("protocol={}", proto.name().to_uppercase()),
                ])
                .collect(),
        );
    }
    steps
}

/// Whether the query output holds a fitting rule for every protocol:
/// enabled, inbound, allow, every profile, this protocol and program.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn fits(query: &str, program: &str, protos: &[Proto]) -> bool {
    protos.iter().all(|proto| {
        query.lines().any(|line| {
            let fields: Vec<&str> = line
                .trim_end_matches('\r')
                .split('\t')
                .map(str::trim)
                .collect();
            matches!(fields.as_slice(), [name, "True", "Inbound", "Allow", "Any", protocol, path]
                if *name == rule_name(*proto)
                    && protocol.eq_ignore_ascii_case(proto.name())
                    && path.eq_ignore_ascii_case(program))
        })
    })
}

#[cfg(target_os = "linux")]
#[path = "firewall_ports_linux.rs"]
mod platform;

/// The Linux helpers `firewall_hook` shares.
#[cfg(target_os = "linux")]
pub(crate) use platform::{program, root, SYSTEMCTL};

#[cfg(windows)]
#[path = "firewall_ports_windows.rs"]
mod platform;

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    pub(super) fn admit(_ports: &[super::Listener]) -> Result<String, super::Refusal> {
        Err(
            String::from("dieses System richtet se-share-server nicht selbst in der Firewall ein")
                .into(),
        )
    }
}

#[cfg(test)]
#[path = "firewall_ports_tests.rs"]
mod tests;
