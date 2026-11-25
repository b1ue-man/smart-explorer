//! The Windows Defender Firewall rules of se's peer programs (user order
//! 2026-11-25: „update, installation, NUTZUNG richten das ein“): one
//! program-bound inbound allow rule per protocol (TCP and UDP) and program
//! of the installation, in every network profile, „Öffentlich“ included.
//! Pure composition and evaluation; `os/windows/firewall.rs` runs the query
//! and the commands.

/// Every rule se reads starts with this (the query's display-name filter).
pub const RULE_PREFIX: &str = "Smart Explorer";
/// The rule up to v0.5.204: UDP only, private and domain networks only, for
/// one program that changed with the caller. It is replaced, never kept.
pub const LEGACY_RULE: &str = "Smart Explorer Share Peer Listener";
/// The programs of an installation that open peer sockets: the background
/// worker runs in either of them, and the screen host worker
/// (`--screen-host-worker`) is the same image started again.
pub const PEER_PROGRAMS: [&str; 2] = ["se.exe", "Smart Explorer.exe"];
const DESCRIPTION: &str =
    "Smart Explorer richtet diese Regel selbst ein: direkte Verbindungen zwischen Geräten";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    pub const BOTH: [Protocol; 2] = [Protocol::Tcp, Protocol::Udp];

    pub fn netsh(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        }
    }
}

/// One rule se wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub program: String,
    pub protocol: Protocol,
}

impl Rule {
    /// `netsh advfirewall firewall add rule …` without the program name.
    pub fn add_args(&self) -> Vec<String> {
        [
            "advfirewall".into(),
            "firewall".into(),
            "add".into(),
            "rule".into(),
            format!("name={}", self.name),
            "dir=in".into(),
            "action=allow".into(),
            format!("program={}", self.program),
            "enable=yes".into(),
            "profile=any".into(),
            format!("protocol={}", self.protocol.netsh()),
            format!("description={DESCRIPTION}"),
        ]
        .into()
    }
}

/// The name of the rule of `program_file` (file name only) for `protocol`.
pub fn rule_name(program_file: &str, protocol: Protocol) -> String {
    format!("Smart Explorer Peer {} ({program_file})", protocol.netsh())
}

/// The rules of the installation folder `dir` for the peer programs that
/// `exists` finds there (a terminal-only installation has no desktop app).
pub fn expected_rules(dir: &str, exists: &dyn Fn(&str) -> bool) -> Vec<Rule> {
    let dir = plain_path(dir);
    let dir = dir.trim_end_matches(['\\', '/']);
    PEER_PROGRAMS
        .iter()
        .filter(|file| exists(file))
        .flat_map(|file| {
            Protocol::BOTH.into_iter().map(move |protocol| Rule {
                name: rule_name(file, protocol),
                program: format!("{dir}\\{file}"),
                protocol,
            })
        })
        .collect()
}

/// A path without the `\\?\` prefix of `canonicalize` (`\\?\UNC\` becomes
/// `\\`): the firewall compares the program path a process runs from.
pub fn plain_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    }
}

/// Network profiles a rule applies in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Profiles {
    pub domain: bool,
    pub private: bool,
    pub public: bool,
}

impl Profiles {
    /// `Profile` of `Get-NetFirewallRule`: `Any` or names joined by `, `.
    pub fn parse(text: &str) -> Self {
        let mut profiles = Self::default();
        for part in text.split(',').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "any" => {
                    profiles = Self {
                        domain: true,
                        private: true,
                        public: true,
                    }
                }
                "domain" => profiles.domain = true,
                "private" => profiles.private = true,
                "public" => profiles.public = true,
                _ => {}
            }
        }
        profiles
    }

    fn all(self) -> bool {
        self.domain && self.private && self.public
    }
}

/// One rule as `Get-NetFirewallRule` and its filters report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observed {
    pub name: String,
    pub enabled: bool,
    pub inbound: bool,
    pub allow: bool,
    pub profiles: Profiles,
    pub protocol: String,
    pub program: String,
    pub local_port: String,
    pub remote_address: String,
}

/// One network profile of the active policy (`Get-NetFirewallProfile
/// -PolicyStore ActiveStore`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileFacts {
    pub name: String,
    pub enabled: bool,
    /// `False` when „Alle eingehenden Verbindungen blockieren“ is set.
    pub inbound_rules_apply: bool,
    /// `False` when a group policy ignores local rules.
    pub local_rules_apply: bool,
}

/// The PowerShell query whose output `parse_query` reads: one tab-separated
/// `R` line per rule whose name starts with `RULE_PREFIX`, one `P` line per
/// profile. The enum values print their English names on every Windows
/// language (no localized `netsh` text is read).
pub fn query_script() -> String {
    // UTF-8 output: a program path with an umlaut (a profile folder such as
    // C:\Users\Jürgen) must compare equal, or every start would ask again.
    format!(
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
         $ErrorActionPreference='SilentlyContinue'; $t=[char]9; \
         Get-NetFirewallRule -DisplayName '{RULE_PREFIX}*' | ForEach-Object {{ \
         $a=$_ | Get-NetFirewallApplicationFilter; $p=$_ | Get-NetFirewallPortFilter; \
         $d=$_ | Get-NetFirewallAddressFilter; \
         'R'+$t+(@($_.DisplayName,$_.Enabled,$_.Direction,$_.Action,$_.Profile,$p.Protocol,\
         $a.Program,(@($p.LocalPort) -join ','),(@($d.RemoteAddress) -join ',')) -join $t) }}; \
         Get-NetFirewallProfile -PolicyStore ActiveStore | ForEach-Object {{ \
         'P'+$t+(@($_.Name,$_.Enabled,$_.AllowInboundRules,$_.AllowLocalFirewallRules) -join $t) }}"
    )
}

fn yes(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("true")
}

/// `True` or `NotConfigured` (the default applies: rules are used).
fn not_false(value: &str) -> bool {
    !value.trim().eq_ignore_ascii_case("false")
}

/// The rules and profiles of the `query_script` output.
pub fn parse_query(text: &str) -> (Vec<Observed>, Vec<ProfileFacts>) {
    let mut rules = Vec::new();
    let mut profiles = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.trim_end_matches('\r').split('\t').collect();
        match fields.as_slice() {
            ["R", name, enabled, direction, action, profile, protocol, program, port, remote] => {
                rules.push(Observed {
                    name: name.trim().to_string(),
                    enabled: yes(enabled),
                    inbound: direction.trim().eq_ignore_ascii_case("inbound"),
                    allow: action.trim().eq_ignore_ascii_case("allow"),
                    profiles: Profiles::parse(profile),
                    protocol: protocol.trim().to_string(),
                    program: program.trim().to_string(),
                    local_port: port.trim().to_string(),
                    remote_address: remote.trim().to_string(),
                })
            }
            ["P", name, enabled, inbound, local] => profiles.push(ProfileFacts {
                name: name.trim().to_string(),
                enabled: yes(enabled),
                inbound_rules_apply: not_false(inbound),
                local_rules_apply: not_false(local),
            }),
            _ => {}
        }
    }
    (rules, profiles)
}

/// Windows paths compare without case and with either slash.
pub fn same_program(left: &str, right: &str) -> bool {
    let norm = |path: &str| path.trim().replace('/', "\\").to_lowercase();
    norm(left) == norm(right)
}

/// What is wrong with `observed` as the rule `want`; `None` when it fits.
pub fn defect(observed: &Observed, want: &Rule) -> Option<&'static str> {
    let any = |value: &str| value.is_empty() || value.eq_ignore_ascii_case("any");
    if !same_program(&observed.program, &want.program) {
        Some("für ein anderes Programm")
    } else if !observed.allow {
        Some("blockiert statt zuzulassen")
    } else if !observed.inbound {
        Some("nicht eingehend")
    } else if !observed.enabled {
        Some("ausgeschaltet")
    } else if !observed
        .protocol
        .eq_ignore_ascii_case(want.protocol.netsh())
    {
        Some("anderes Protokoll")
    } else if !observed.profiles.all() {
        Some("nicht in allen Netzprofilen")
    } else if !any(&observed.local_port) || !any(&observed.remote_address) {
        Some("auf Ports oder Adressen beschränkt")
    } else {
        None
    }
}

/// One firewall change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Every rule of this name (the legacy rule only).
    DeleteName(String),
    /// The rules of this name for exactly this program.
    DeleteFor {
        name: String,
        program: String,
    },
    Add(Rule),
}

impl Step {
    /// The `netsh` arguments of this step.
    pub fn args(&self) -> Vec<String> {
        let delete = |name: &str| -> Vec<String> {
            ["advfirewall", "firewall", "delete", "rule"]
                .into_iter()
                .map(String::from)
                .chain([format!("name={name}")])
                .collect()
        };
        match self {
            Self::DeleteName(name) => delete(name),
            Self::DeleteFor { name, program } => {
                let mut args = delete(name);
                args.push(format!("program={program}"));
                args
            }
            Self::Add(rule) => rule.add_args(),
        }
    }
}

/// The changes that make the observed rules what `expected` asks for, and
/// one text per missing or outdated rule.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub findings: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// A rule of an expected name for another program (another installation of
/// this account or machine, or a former folder of this one) stays; this
/// program gets its own. The legacy rule goes with the first change: alone it
/// raises no confirmation, since the expected rules admit everything it did.
pub fn plan(expected: &[Rule], observed: &[Observed]) -> Plan {
    let mut out = Plan::default();
    for rule in expected {
        let mine: Vec<&Observed> = observed
            .iter()
            .filter(|o| o.name == rule.name && same_program(&o.program, &rule.program))
            .collect();
        let wrong = mine.iter().find_map(|o| defect(o, rule));
        if !mine.is_empty() && wrong.is_none() {
            continue;
        }
        if !mine.is_empty() {
            out.steps.push(Step::DeleteFor {
                name: rule.name.clone(),
                program: rule.program.clone(),
            });
        }
        out.steps.push(Step::Add(rule.clone()));
        out.findings.push(format!(
            "„{}“ für {} {}",
            rule.name,
            rule.program,
            wrong.unwrap_or("fehlt")
        ));
    }
    if out.steps.is_empty() {
        return out;
    }
    if let Some(legacy) = observed.iter().find(|o| o.name == LEGACY_RULE) {
        out.steps.insert(0, Step::DeleteName(LEGACY_RULE.into()));
        out.findings.insert(
            0,
            format!(
                "veraltete Regel „{LEGACY_RULE}“ ({} für {} in {}) wird ersetzt",
                legacy.protocol,
                legacy.program,
                profile_names(legacy.profiles)
            ),
        );
    }
    out
}

fn quote_ps(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// The script of the elevated step: no module loads, only `netsh` by its
/// full path; a failed add ends with exit code 1, deletes may find nothing.
pub fn elevated_script(netsh: &str, steps: &[Step]) -> String {
    let mut script = format!(
        "$PSModuleAutoLoadingPreference='None'; $f=0; $n={}; ",
        quote_ps(netsh)
    );
    for step in steps {
        let args: Vec<String> = step.args().iter().map(|arg| quote_ps(arg)).collect();
        script.push_str(&format!("$null = & $n {}; ", args.join(" ")));
        if matches!(step, Step::Add(_)) {
            script.push_str("if ($LASTEXITCODE -ne 0) { $f=1 }; ");
        }
    }
    script.push_str("exit $f");
    script
}

fn profile_names(profiles: Profiles) -> String {
    let names: Vec<&str> = [
        (profiles.domain, "Domäne"),
        (profiles.private, "Privat"),
        (profiles.public, "Öffentlich"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    if names.is_empty() {
        "keinem Netzprofil".into()
    } else {
        names.join(", ")
    }
}

fn german_profile(name: &str) -> &str {
    match name.to_ascii_lowercase().as_str() {
        "domain" => "Domäne",
        "private" => "Privat",
        "public" => "Öffentlich",
        _ => name,
    }
}

/// The status line of rules that fit; a profile whose own setting keeps
/// every rule out is named (se does not change that setting).
pub fn status_line(expected: &[Rule], profiles: &[ProfileFacts]) -> String {
    let mut programs: Vec<&str> = Vec::new();
    for rule in expected {
        let file = rule.program.rsplit('\\').next().unwrap_or(&rule.program);
        if !programs.contains(&file) {
            programs.push(file);
        }
    }
    let mut line = format!(
        "Firewall-Regeln aktiv: eingehend TCP und UDP für {} in allen Netzprofilen (Domäne, Privat, Öffentlich)",
        programs.join(" und ")
    );
    for profile in profiles.iter().filter(|profile| profile.enabled) {
        let name = german_profile(&profile.name);
        if !profile.inbound_rules_apply {
            line.push_str(&format!(
                "; Achtung: im Profil {name} ist „Alle eingehenden Verbindungen blockieren“ eingestellt, dort wirkt keine Regel"
            ));
        } else if !profile.local_rules_apply {
            line.push_str(&format!(
                "; Achtung: im Profil {name} lässt eine Gruppenrichtlinie keine lokalen Regeln zu"
            ));
        }
    }
    line
}

#[cfg(test)]
#[path = "firewall_windows_plan_tests.rs"]
mod tests;
