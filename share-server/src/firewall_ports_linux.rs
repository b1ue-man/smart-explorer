//! The Linux part of `firewall_ports`: ufw, firewalld, nftables and legacy
//! iptables, in the order their rules decide (ufw and firewalld manage their
//! own nftables or iptables rules). Without root, nothing is asked while no
//! firewall service is visible.
use super::{last_line, run, stdout, Listener, Privilege, Refusal, MARK};
use std::path::Path;
use std::process::Output;

const UFW: [&str; 2] = ["/usr/sbin/ufw", "/sbin/ufw"];
const FIREWALL_CMD: [&str; 2] = ["/usr/bin/firewall-cmd", "/bin/firewall-cmd"];
const NFT: [&str; 2] = ["/usr/sbin/nft", "/sbin/nft"];
const IPTABLES: [[&str; 2]; 2] = [
    ["/usr/sbin/iptables", "/sbin/iptables"],
    ["/usr/sbin/ip6tables", "/sbin/ip6tables"],
];
pub(crate) const SYSTEMCTL: [&str; 2] = ["/usr/bin/systemctl", "/bin/systemctl"];

pub(crate) fn program(candidates: &[&'static str]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|path| Path::new(path).is_file())
}

fn own(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}

/// `ufw status`: `Status: active`.
pub(super) fn ufw_active(status: &str) -> bool {
    status
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case("status: active"))
}

/// `/etc/ufw/ufw.conf`: `ENABLED=yes`.
pub(super) fn ufw_enabled(conf: &str) -> bool {
    conf.lines().map(str::trim).any(|line| {
        !line.starts_with('#')
            && line.split_once('=').is_some_and(|(key, value)| {
                key.trim() == "ENABLED" && value.trim().trim_matches('"') == "yes"
            })
    })
}

/// Zone names of `firewall-cmd --get-active-zones` and the default zone.
pub(super) fn firewalld_zones(active: &str, default: &str) -> Vec<String> {
    let mut zones: Vec<String> = Vec::new();
    for name in active
        .lines()
        .filter(|line| !line.starts_with([' ', '\t']))
        .chain([default])
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.contains(char::is_whitespace))
    {
        if !zones.iter().any(|zone| zone == name) {
            zones.push(name.to_string());
        }
    }
    zones
}

/// The input base chains (inet, ip, ip6) of `nft list ruleset` that drop
/// what no rule accepts, with their rule lines.
pub(super) fn nft_filtering_chains(ruleset: &str) -> Vec<(String, String, String, Vec<String>)> {
    let mut found = Vec::new();
    let mut table: Option<(String, String)> = None;
    let mut chain: Option<(String, Vec<String>)> = None;
    let (mut input, mut drops, mut other) = (false, false, 0usize);
    for line in ruleset.lines().map(str::trim) {
        if other > 0 {
            other = other + usize::from(line.ends_with('{')) - usize::from(line == "}");
        } else if let Some(rest) = line.strip_prefix("table ") {
            let words: Vec<&str> = rest.trim_end_matches('{').split_whitespace().collect();
            table = (words.len() == 2).then(|| (words[0].to_string(), words[1].to_string()));
        } else if let Some(rest) = line.strip_prefix("chain ") {
            chain = Some((rest.trim_end_matches('{').trim().to_string(), Vec::new()));
            (input, drops) = (false, false);
        } else if line == "}" {
            let Some((name, rules)) = chain.take() else {
                table = None;
                continue;
            };
            let Some((family, table_name)) = &table else {
                continue;
            };
            let unconditional = rules.iter().any(|rule| {
                let words: Vec<&str> = rule.split_whitespace().collect();
                let verdict = match words.as_slice() {
                    ["counter", "packets", _, "bytes", _, rest @ ..] | ["counter", rest @ ..] => {
                        rest.join(" ")
                    }
                    rest => rest.join(" "),
                };
                verdict == "drop" || verdict == "reject" || verdict.starts_with("reject with ")
            });
            let family_ok = matches!(family.as_str(), "inet" | "ip" | "ip6");
            if input && family_ok && (drops || unconditional) {
                found.push((family.clone(), table_name.clone(), name, rules));
            }
        } else if let Some((_, rules)) = chain.as_mut() {
            if line.starts_with("type ") && line.contains(" hook input ") {
                input = true;
                drops = line.contains("policy drop");
            } else if !line.is_empty() && !line.starts_with('#') {
                rules.push(line.to_string());
            }
        } else if table.is_some() && line.ends_with('{') {
            other = 1;
        }
    }
    found
}

/// `nft insert rule …` that accepts `port` at the head of the chain.
pub(super) fn nft_insert(family: &str, table: &str, chain: &str, port: &Listener) -> Vec<String> {
    let number = port.bind.port().to_string();
    own(&[
        "insert",
        "rule",
        family,
        table,
        chain,
        port.proto.name(),
        "dport",
        &number,
        "accept",
        "comment",
        &format!("\"{MARK}\""),
    ])
}

/// `iptables -S INPUT`: policy DROP or an unconditional DROP/REJECT.
pub(super) fn iptables_filters(rules: &str) -> bool {
    rules.lines().map(str::trim).any(|line| {
        line == "-P INPUT DROP"
            || line
                .strip_prefix("-A INPUT ")
                .is_some_and(|rest| rest == "-j DROP" || rest.starts_with("-j REJECT"))
    })
}

/// `iptables -C|-I INPUT …` for `port` with the server's mark.
pub(super) fn iptables_args(check: bool, port: &Listener) -> Vec<String> {
    let number = port.bind.port().to_string();
    own(&[
        if check { "-C" } else { "-I" },
        "INPUT",
        "-p",
        port.proto.name(),
        "--dport",
        &number,
        "-m",
        "comment",
        "--comment",
        MARK,
        "-j",
        "ACCEPT",
    ])
}

pub(crate) fn root() -> bool {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    super::effective_uid(&status) == Some(0)
}

fn service_active(name: &str) -> bool {
    program(&SYSTEMCTL).is_some_and(|systemctl| {
        run(&own(&[systemctl, "is-active", "--quiet", name])).is_ok_and(|out| out.status.success())
    })
}

/// A firewall any account can see: ufw enabled in its configuration, or a
/// firewalld, nftables or iptables service.
fn visible() -> bool {
    std::fs::read_to_string("/etc/ufw/ufw.conf").is_ok_and(|conf| ufw_enabled(&conf))
        || [
            "firewalld",
            "nftables",
            "iptables",
            "ip6tables",
            "netfilter-persistent",
        ]
        .into_iter()
        .any(service_active)
}

/// Root, or sudo after one check (that asks in a terminal).
fn privilege() -> Result<Privilege, String> {
    use std::io::IsTerminal;
    if root() {
        return Ok(Privilege::Root);
    }
    let privilege = if std::io::stdin().is_terminal() {
        Privilege::SudoAsk
    } else {
        Privilege::SudoQuiet
    };
    let probe = run(&privilege.argv("true", &[]))
        .map_err(|error| format!("weder root noch sudo: {error}"))?;
    if probe.status.success() {
        Ok(privilege)
    } else {
        Err(format!("Root-Rechte fehlen (sudo: {})", last_line(&probe)))
    }
}

fn must(privilege: Privilege, tool: &str, args: &[String]) -> Result<(), String> {
    let output = run(&privilege.argv(tool, args)).map_err(|error| format!("{tool}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("{tool}: {}", last_line(&output)))
    }
}

fn read(privilege: Privilege, tool: &str, args: &[&str]) -> Option<Output> {
    run(&privilege.argv(tool, &own(args)))
        .ok()
        .filter(|output| output.status.success())
}

pub(super) fn admit(ports: &[Listener]) -> Result<String, Refusal> {
    if !root() && !visible() {
        return Ok("keine aktive Firewall sichtbar (ufw, firewalld, nftables, iptables); nichts freizugeben".into());
    }
    let privilege = privilege().map_err(Refusal::rights)?;
    let list = super::labels(ports);
    if let Some(ufw) = program(&UFW) {
        if read(privilege, ufw, &["status"]).is_some_and(|out| ufw_active(&stdout(&out))) {
            for port in ports {
                let comment = format!("se-share-server: {}", port.purpose);
                must(
                    privilege,
                    ufw,
                    &own(&["allow", &port.label(), "comment", &comment]),
                )?;
            }
            return Ok(format!("ufw: eingehend freigegeben {list}"));
        }
    }
    if let Some(firewall_cmd) = program(&FIREWALL_CMD) {
        if read(privilege, firewall_cmd, &["--state"]).is_some() {
            let text = |args: &[&str]| {
                read(privilege, firewall_cmd, args)
                    .map(|out| stdout(&out))
                    .unwrap_or_default()
            };
            let zones = firewalld_zones(
                &text(&["--get-active-zones"]),
                &text(&["--get-default-zone"]),
            );
            for zone in &zones {
                let adds: Vec<String> = ports
                    .iter()
                    .map(|port| format!("--add-port={}", port.label()))
                    .collect();
                for permanent in [true, false] {
                    let mut args = Vec::new();
                    if permanent {
                        args.push("--permanent".to_string());
                    }
                    args.push(format!("--zone={zone}"));
                    args.extend(adds.iter().cloned());
                    must(privilege, firewall_cmd, &args)?;
                }
            }
            return Ok(format!(
                "firewalld (Zonen {}): eingehend freigegeben {list}",
                zones.join(", ")
            ));
        }
    }
    if let Some(nft) = program(&NFT) {
        let ruleset = read(privilege, nft, &["list", "ruleset"])
            .map(|out| stdout(&out))
            .unwrap_or_default();
        let chains = nft_filtering_chains(&ruleset);
        if !chains.is_empty() {
            for (family, table, chain, rules) in &chains {
                for port in ports {
                    let wanted = format!("{} dport {} accept", port.proto.name(), port.bind.port());
                    let marked = format!("comment \"{MARK}\"");
                    if !rules
                        .iter()
                        .any(|rule| rule.contains(&wanted) && rule.contains(&marked))
                    {
                        must(privilege, nft, &nft_insert(family, table, chain, port))?;
                    }
                }
            }
            return Ok(format!(
                "nftables: eingehend freigegeben {list} (bis zum nächsten Laden des Regelsatzes; se-share-server prüft bei jedem Start)"
            ));
        }
    }
    let mut legacy = false;
    for binary in IPTABLES.iter().filter_map(|candidates| program(candidates)) {
        let is_legacy = read(privilege, binary, &["--version"])
            .is_some_and(|out| !stdout(&out).contains("nf_tables"));
        let filters = read(privilege, binary, &["-S", "INPUT"])
            .is_some_and(|out| iptables_filters(&stdout(&out)));
        if is_legacy && filters {
            legacy = true;
            for port in ports {
                let present = run(&privilege.argv(binary, &iptables_args(true, port)))
                    .is_ok_and(|out| out.status.success());
                if !present {
                    must(privilege, binary, &iptables_args(false, port))?;
                }
            }
        }
    }
    Ok(if legacy {
        format!("iptables: eingehend freigegeben {list} (bis zum Neustart; se-share-server prüft bei jedem Start)")
    } else {
        "keine filternde Firewall (ufw, firewalld, nftables, iptables geprüft); nichts freizugeben"
            .into()
    })
}
