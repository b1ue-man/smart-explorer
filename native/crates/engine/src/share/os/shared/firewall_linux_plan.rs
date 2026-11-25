//! The host firewalls of Linux (ufw, firewalld, nftables, iptables) and the
//! ports se admits in them (user orders 2026-11-25). Linux firewalls filter
//! by port, not by program: se admits its fixed listening ports. Its peer
//! sockets on ports the system picks (Iroh's UDP socket, the screen's direct
//! UDP and TCP paths, where both devices send first and TCP meets by
//! simultaneous open) get through as replies to its own outgoing packets,
//! which the connection tracking of every one of these firewalls accepts.
//! Pure parsing and command composition; `os/linux_os/firewall.rs` runs them.

/// The comment that marks se's own nftables and iptables rules.
pub const MARK: &str = "smart-explorer";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    pub fn name(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

/// One inbound port se admits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Port {
    pub number: u16,
    pub protocol: Protocol,
    pub purpose: &'static str,
}

impl Port {
    /// `5353/udp`, the form of ufw, firewalld and the record.
    pub fn label(self) -> String {
        format!("{}/{}", self.number, self.protocol.name())
    }
}

/// The fixed inbound ports of a device. A direct listener on a fixed port
/// belongs here; one on a port the system picks reaches the firewall only as
/// a reply to se's own packets.
pub const DEVICE_PORTS: &[Port] = &[Port {
    number: 5353,
    protocol: Protocol::Udp,
    purpose: "mDNS: gekoppelte Geräte im lokalen Netz finden",
}];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Ufw,
    Firewalld,
    Nftables,
    Iptables,
}

impl Backend {
    pub const ALL: [Backend; 4] = [
        Backend::Ufw,
        Backend::Firewalld,
        Backend::Nftables,
        Backend::Iptables,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Ufw => "ufw",
            Self::Firewalld => "firewalld",
            Self::Nftables => "nftables",
            Self::Iptables => "iptables",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|backend| backend.name() == name)
    }

    /// ufw and firewalld keep their rules across a restart; rules added to an
    /// nftables or iptables ruleset live until the next reboot or reload.
    pub fn persistent(self) -> bool {
        matches!(self, Self::Ufw | Self::Firewalld)
    }
}

/// `ufw status` (root): `Status: active`.
pub fn ufw_active(status: &str) -> bool {
    status
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case("status: active"))
}

/// `/etc/ufw/ufw.conf` (readable by everyone): `ENABLED=yes`.
pub fn ufw_enabled(conf: &str) -> bool {
    conf.lines().any(|line| {
        let line = line.trim();
        !line.starts_with('#')
            && line.split_once('=').is_some_and(|(key, value)| {
                key.trim() == "ENABLED"
                    && value.trim().trim_matches('"').eq_ignore_ascii_case("yes")
            })
    })
}

/// `firewall-cmd --get-active-zones` (zone names at the line start, their
/// interfaces indented below) and the default zone, without repeats.
pub fn firewalld_zones(active_zones: &str, default_zone: &str) -> Vec<String> {
    let mut zones: Vec<String> = Vec::new();
    let names = active_zones
        .lines()
        .filter(|line| !line.starts_with([' ', '\t']))
        .chain([default_zone])
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.contains(char::is_whitespace));
    for name in names {
        if !zones.iter().any(|zone| zone == name) {
            zones.push(name.to_string());
        }
    }
    zones
}

/// One nftables base chain on the input hook.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NftChain {
    pub family: String,
    pub table: String,
    pub chain: String,
    pub policy_drop: bool,
    pub rules: Vec<String>,
}

impl NftChain {
    /// Drops what no rule accepts: policy drop, or a rule that drops or
    /// rejects without a match (`counter … drop`, `reject with …`).
    pub fn filters(&self) -> bool {
        self.policy_drop
            || self.rules.iter().any(|rule| {
                let verdict = verdict(rule);
                verdict == "drop" || verdict == "reject" || verdict.starts_with("reject with ")
            })
    }

    pub fn admits(&self, port: Port) -> bool {
        let wanted = format!("{} dport {} accept", port.protocol.name(), port.number);
        let marker = format!("comment \"{MARK}\"");
        self.rules
            .iter()
            .any(|rule| rule.contains(&wanted) && rule.contains(&marker))
    }

    /// `nft insert rule …`: se's accept rule at the head of this chain.
    pub fn insert_args(&self, port: Port) -> Vec<String> {
        [
            "insert",
            "rule",
            &self.family,
            &self.table,
            &self.chain,
            port.protocol.name(),
            "dport",
            &port.number.to_string(),
            "accept",
            "comment",
            &format!("\"{MARK}\""),
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }
}

/// The rule without a leading `counter` (`counter packets N bytes M drop`).
fn verdict(rule: &str) -> String {
    let words: Vec<&str> = rule.split_whitespace().collect();
    let rest = match words.as_slice() {
        ["counter", "packets", _, "bytes", _, rest @ ..] => rest,
        ["counter", rest @ ..] => rest,
        rest => rest,
    };
    rest.join(" ")
}

/// The input base chains of `nft list ruleset` (families inet, ip and ip6).
pub fn nft_input_chains(ruleset: &str) -> Vec<NftChain> {
    let mut chains = Vec::new();
    let mut table: Option<(String, String)> = None;
    let mut current: Option<NftChain> = None;
    let mut input = false;
    // Open `set`, `map`, `flowtable` and other blocks of a table.
    let mut other = 0usize;
    for line in ruleset.lines().map(str::trim) {
        if other > 0 {
            other += usize::from(line.ends_with('{'));
            other -= usize::from(line == "}");
        } else if current.is_none()
            && table.is_some()
            && line.ends_with('{')
            && !line.starts_with("chain ")
        {
            other = 1;
        } else if let Some(rest) = line.strip_prefix("table ") {
            let mut words = rest.trim_end_matches('{').split_whitespace();
            table = match (words.next(), words.next()) {
                (Some(family), Some(name)) => Some((family.into(), name.into())),
                _ => None,
            };
        } else if let Some(rest) = line.strip_prefix("chain ") {
            let (family, name) = table.clone().unwrap_or_default();
            current = Some(NftChain {
                family,
                table: name,
                chain: rest.trim_end_matches('{').trim().to_string(),
                ..NftChain::default()
            });
            input = false;
        } else if line == "}" {
            if let Some(chain) = current.take() {
                if input && matches!(chain.family.as_str(), "inet" | "ip" | "ip6") {
                    chains.push(chain);
                }
            } else {
                table = None;
            }
        } else if let Some(chain) = current.as_mut() {
            if line.starts_with("type ") && line.contains(" hook input ") {
                input = true;
                chain.policy_drop = line.contains("policy drop");
            } else if !line.is_empty() && !line.starts_with('#') {
                chain.rules.push(line.to_string());
            }
        }
    }
    chains
}

/// `iptables --version`: the legacy variant keeps rules nftables does not
/// show (`iptables v1.8.9 (legacy)`; before 1.8 without a suffix).
pub fn iptables_legacy(version: &str) -> bool {
    version.contains("iptables") && !version.contains("nf_tables")
}

/// `iptables -S INPUT`: policy DROP or an unconditional DROP/REJECT.
pub fn iptables_filters(rules: &str) -> bool {
    rules.lines().map(str::trim).any(|line| {
        line == "-P INPUT DROP"
            || line
                .strip_prefix("-A INPUT ")
                .is_some_and(|rest| rest == "-j DROP" || rest.starts_with("-j REJECT"))
    })
}

fn iptables_rule(port: Port) -> Vec<String> {
    let number = port.number.to_string();
    [
        "-p",
        port.protocol.name(),
        "--dport",
        &number,
        "-m",
        "comment",
        "--comment",
        MARK,
        "-j",
        "ACCEPT",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// `iptables -C INPUT …` (exit 0 when se's rule exists) and `-I INPUT …`.
pub fn iptables_args(check: bool, port: Port) -> Vec<String> {
    let mut args = vec![
        if check { "-C" } else { "-I" }.to_string(),
        "INPUT".to_string(),
    ];
    args.extend(iptables_rule(port));
    args
}

/// `ufw allow 5353/udp comment …` (ufw skips a rule it has).
pub fn ufw_args(port: Port) -> Vec<String> {
    vec![
        "allow".into(),
        port.label(),
        "comment".into(),
        format!("Smart Explorer: {}", port.purpose),
    ]
}

/// The permanent and the runtime `firewall-cmd` call for one zone.
pub fn firewalld_args(zone: &str, ports: &[Port]) -> [Vec<String>; 2] {
    let adds: Vec<String> = ports
        .iter()
        .map(|port| format!("--add-port={}", port.label()))
        .collect();
    let call = |permanent: bool| {
        let mut args = Vec::new();
        if permanent {
            args.push("--permanent".to_string());
        }
        args.push(format!("--zone={zone}"));
        args.extend(adds.iter().cloned());
        args
    };
    [call(true), call(false)]
}

/// The record the root step leaves: the firewall it found (`none`: none
/// filters), the ports it admitted and the firewall services any account
/// sees running (`seen`), so a later use needs no root while that set stays.
pub fn record_text(backend: Option<Backend>, ports: &[Port], seen: &[Backend]) -> String {
    let labels: Vec<String> = ports.iter().map(|port| port.label()).collect();
    let seen: Vec<&str> = seen.iter().map(|backend| backend.name()).collect();
    format!(
        "backend={}\nports={}\nseen={}\n",
        backend.map_or("none", Backend::name),
        labels.join(","),
        seen.join(",")
    )
}

/// A parsed record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// `None`: no filtering firewall was found.
    pub backend: Option<Backend>,
    pub ports: Vec<String>,
    pub seen: Vec<Backend>,
}

fn list(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
}

impl Record {
    pub fn parse(text: &str) -> Option<Self> {
        let mut backend = None;
        let mut ports = Vec::new();
        let mut seen = Vec::new();
        for line in text.lines() {
            match line.trim().split_once('=') {
                Some(("backend", "none")) => backend = Some(None),
                Some(("backend", name)) => backend = Some(Some(Backend::parse(name)?)),
                Some(("ports", value)) => ports = list(value).map(String::from).collect(),
                Some(("seen", value)) => {
                    seen = list(value)
                        .map(Backend::parse)
                        .collect::<Option<Vec<_>>>()?
                }
                _ => {}
            }
        }
        Some(Self {
            backend: backend?,
            ports,
            seen,
        })
    }

    /// Whether this record still answers for `ports` while the services in
    /// `visible` run: the same set of running firewalls, and every port
    /// admitted (a record of no filtering firewall needs none).
    pub fn fits(&self, ports: &[Port], visible: &[Backend]) -> bool {
        let same = self.seen.len() == visible.len()
            && visible.iter().all(|backend| self.seen.contains(backend));
        same && (self.backend.is_none()
            || ports
                .iter()
                .all(|port| self.ports.iter().any(|label| *label == port.label())))
    }
}

/// What the pkexec call of the root step reported (`firewall_state=` line,
/// exit code, pkexec's own error output: 126 the dialog was dismissed, 127
/// not authorized or no polkit agent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    Done,
    Declined(String),
    Failed(String),
}

pub fn step_outcome(code: Option<i32>, stdout: &str, stderr: &str, missing: &str) -> StepOutcome {
    let state = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("firewall_state="));
    let declined = || {
        StepOutcome::Declined(format!(
            "Die Freigabe von {missing} in der Firewall wurde in der Systemabfrage abgelehnt; se fragt beim nächsten Start erneut"
        ))
    };
    match (code, state) {
        (_, Some(state)) if state.starts_with("done") => StepOutcome::Done,
        (_, Some(state)) => StepOutcome::Failed(format!(
            "Firewall-Freigabe fehlgeschlagen: {}; nicht freigegeben: {missing}",
            state.trim_start_matches("failed:")
        )),
        (Some(126), None) => declined(),
        (Some(127), None) if stderr.contains("No authentication agent") => StepOutcome::Failed(format!(
            "{missing} kann nicht freigegeben werden: in dieser Sitzung läuft kein Polkit-Agent, der nach der Freigabe fragt"
        )),
        (Some(127), None) => declined(),
        (_, None) if stdout.contains("unknown_system_step") => StepOutcome::Failed(format!(
            "dieses se kennt den Firewall-Schritt nicht; nicht freigegeben: {missing}"
        )),
        (code, None) => StepOutcome::Failed(format!(
            "Firewall-Schritt endete mit {}; nicht freigegeben: {missing}",
            code.map_or("einem Signal".to_string(), |code| format!("Code {code}"))
        )),
    }
}

/// The status line of an admitted port set.
pub fn status_line(backend: Option<Backend>, ports: &[Port]) -> String {
    match backend {
        None => {
            "Firewall: keine filternde Firewall aktiv (geprüft: ufw, firewalld, nftables, iptables)"
                .into()
        }
        Some(backend) => {
            let list: Vec<String> = ports
                .iter()
                .map(|port| format!("{} ({})", port.label(), port.purpose))
                .collect();
            format!(
                "Firewall ({}): eingehend freigegeben {}; die Direktwege über UDP und TCP (beide Geräte senden zuerst, auf Ports, die das System wählt) lässt die Zustandsverfolgung der Firewall als Antwort zu",
                backend.name(),
                list.join(", ")
            )
        }
    }
}

#[cfg(test)]
#[path = "firewall_linux_plan_tests.rs"]
mod tests;
