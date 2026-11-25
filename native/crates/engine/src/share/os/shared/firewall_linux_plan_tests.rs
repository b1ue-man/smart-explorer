use super::*;

const MDNS: Port = DEVICE_PORTS[0];
const LISTENER: Port = Port {
    number: 51830,
    protocol: Protocol::Tcp,
    purpose: "Prüfport",
};

const RULESET: &str = r#"table inet filter {
	set allowed {
		type ipv4_addr
		elements = { 10.0.0.1 }
	}
	chain input {
		type filter hook input priority filter; policy drop;
		ct state established,related accept
		tcp dport 22 drop
	}
	chain output {
		type filter hook output priority filter; policy accept;
	}
}
table ip filter {
	chain INPUT {
		type filter hook input priority filter; policy accept;
		counter packets 12 bytes 3456 reject with icmp type port-unreachable
	}
}
table ip6 open {
	chain in6 {
		type filter hook input priority filter; policy accept;
		tcp dport 22 drop
	}
}
table netdev edge {
	chain ingress {
		type filter hook ingress device "eth0" priority filter; policy drop;
	}
}
"#;

/// ufw counts when `ufw status` says active or its readable configuration
/// enables it; firewalld's active zones and default zone without repeats;
/// the nftables input base chains of inet, ip and ip6 that drop what no rule
/// accepts (policy drop or an unconditional drop/reject), also behind a set
/// block of the table; iptables with policy DROP or a final REJECT, and only
/// its legacy variant keeps rules nftables does not show.
#[test]
fn firewall_rules_task_linux_detects_active_firewalls() {
    assert!(ufw_active(
        "Status: active\n\nTo                         Action      From\n"
    ));
    assert!(!ufw_active("Status: inactive\n"));
    assert!(ufw_enabled("# comment\nENABLED=yes\nLOGLEVEL=low\n"));
    assert!(ufw_enabled("ENABLED=\"yes\"\n"));
    assert!(!ufw_enabled("ENABLED=no\n#ENABLED=yes\n"));

    assert_eq!(
        firewalld_zones(
            "public\n  interfaces: eth0\ndocker\n  interfaces: docker0\n",
            "public\n"
        ),
        ["public", "docker"]
    );
    assert_eq!(
        firewalld_zones("", "FedoraWorkstation\n"),
        ["FedoraWorkstation"]
    );

    let chains = nft_input_chains(RULESET);
    let names: Vec<(&str, &str, &str, bool)> = chains
        .iter()
        .map(|chain| {
            (
                chain.family.as_str(),
                chain.table.as_str(),
                chain.chain.as_str(),
                chain.filters(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            ("inet", "filter", "input", true),
            ("ip", "filter", "INPUT", true),
            ("ip6", "open", "in6", false),
        ]
    );
    assert_eq!(chains[0].rules.len(), 2);

    assert!(iptables_filters(
        "-P INPUT DROP\n-A INPUT -i lo -j ACCEPT\n"
    ));
    assert!(iptables_filters(
        "-P INPUT ACCEPT\n-A INPUT -j REJECT --reject-with icmp-port-unreachable\n"
    ));
    assert!(!iptables_filters(
        "-P INPUT ACCEPT\n-A INPUT -p tcp --dport 22 -j DROP\n"
    ));
    // ufw active, but this family's INPUT never reaches ufw's chains.
    let own_rules =
        "-P INPUT DROP\n-A INPUT -i lo -j ACCEPT\n-A INPUT -p tcp -m tcp --dport 22 -j ACCEPT\n";
    assert!(bypasses_ufw(own_rules));
    assert!(!bypasses_ufw(
        "-P INPUT DROP\n-A INPUT -j ufw-before-logging-input\n-A INPUT -j ufw-before-input\n"
    ));
    assert!(!bypasses_ufw(
        "-P INPUT DROP\n-A INPUT -j ufw6-before-input\n"
    ));
    assert!(!bypasses_ufw("-P INPUT ACCEPT\n"));
    assert!(iptables_legacy("iptables v1.8.9 (legacy)"));
    assert!(iptables_legacy("iptables v1.6.1"));
    assert!(!iptables_legacy("iptables v1.8.9 (nf_tables)"));
}

/// The admission commands per firewall: ufw per port with se's comment,
/// firewalld permanently and at runtime per zone, an nftables accept rule
/// with se's mark at the head of a filtering chain (recognized once it is
/// there), iptables check and insert of the same rule.
#[test]
fn firewall_rules_task_linux_admission_commands() {
    assert_eq!(MDNS.label(), "5353/udp");
    assert_eq!(
        ufw_args(MDNS),
        [
            "allow",
            "5353/udp",
            "comment",
            format!("Smart Explorer: {}", MDNS.purpose).as_str()
        ]
    );
    let [permanent, runtime] = firewalld_args("public", &[MDNS, LISTENER]);
    assert_eq!(
        permanent,
        [
            "--permanent",
            "--zone=public",
            "--add-port=5353/udp",
            "--add-port=51830/tcp"
        ]
    );
    assert_eq!(
        runtime,
        [
            "--zone=public",
            "--add-port=5353/udp",
            "--add-port=51830/tcp"
        ]
    );

    let mut chain = nft_input_chains(RULESET).remove(0);
    assert_eq!(
        chain.insert_args(LISTENER),
        [
            "insert",
            "rule",
            "inet",
            "filter",
            "input",
            "tcp",
            "dport",
            "51830",
            "accept",
            "comment",
            "\"smart-explorer\""
        ]
    );
    assert!(!chain.admits(MDNS));
    chain
        .rules
        .insert(0, "udp dport 5353 accept comment \"smart-explorer\"".into());
    assert!(chain.admits(MDNS));
    assert!(!chain.admits(LISTENER));

    assert_eq!(
        iptables_args(true, MDNS),
        [
            "-C",
            "INPUT",
            "-p",
            "udp",
            "--dport",
            "5353",
            "-m",
            "comment",
            "--comment",
            "smart-explorer",
            "-j",
            "ACCEPT"
        ]
    );
    assert_eq!(iptables_args(false, MDNS)[0], "-I");
}

/// The root step's record answers for exactly the firewalls running when it
/// was written and the ports it admitted: a new firewall, a missing port or
/// an unknown backend asks again; a record of no filtering firewall needs no
/// port. The pkexec result becomes done, declined (dialog dismissed, not
/// authorized) or failed with the exact reason and the missing ports.
#[test]
fn firewall_rules_task_linux_record_and_outcome() {
    let seen = [Backend::Ufw, Backend::Nftables];
    let text = record_text(Some(Backend::Ufw), &[MDNS], &seen);
    assert_eq!(text, "backend=ufw\nports=5353/udp\nseen=ufw,nftables\n");
    let record = Record::parse(&text).expect("record");
    assert!(record.fits(&[MDNS], &[Backend::Nftables, Backend::Ufw]));
    assert!(!record.fits(&[MDNS], &[Backend::Ufw]));
    assert!(!record.fits(&[MDNS, LISTENER], &seen));
    let none = Record::parse(&record_text(None, &[MDNS], &[Backend::Nftables])).expect("none");
    assert!(none.fits(&[MDNS, LISTENER], &[Backend::Nftables]));
    assert_eq!(Record::parse("backend=pf\nports=\n"), None);
    assert_eq!(Record::parse("ports=5353/udp\n"), None);
    assert!(Backend::Ufw.persistent() && !Backend::Nftables.persistent());

    let missing = "5353/udp";
    assert_eq!(
        step_outcome(Some(0), "firewall_state=done:ufw\n", "", missing),
        StepOutcome::Done
    );
    assert_eq!(
        step_outcome(Some(70), "firewall_state=failed:ufw: ERROR\n", "", missing),
        StepOutcome::Failed(
            "Firewall-Freigabe fehlgeschlagen: ufw: ERROR; nicht freigegeben: 5353/udp".into()
        )
    );
    assert!(
        matches!(step_outcome(Some(126), "", "", missing), StepOutcome::Declined(reason) if reason.contains("5353/udp") && reason.contains("erneut"))
    );
    assert!(matches!(
        step_outcome(Some(127), "", "Error executing command as another user: No authentication agent found.", missing),
        StepOutcome::Failed(reason) if reason.contains("kein Polkit-Agent")
    ));
    assert!(matches!(
        step_outcome(Some(127), "", "Not authorized", missing),
        StepOutcome::Declined(_)
    ));
    assert!(matches!(
        step_outcome(Some(1), "state=failed:unknown_system_step\n", "", missing),
        StepOutcome::Failed(reason) if reason.contains("kennt den Firewall-Schritt nicht")
    ));
    assert!(matches!(
        step_outcome(None, "", "", missing),
        StepOutcome::Failed(reason) if reason.contains("einem Signal")
    ));
    assert!(status_line(Some(Backend::Firewalld), &[MDNS])
        .starts_with("Firewall (firewalld): eingehend freigegeben 5353/udp (mDNS"));
    assert!(status_line(None, &[MDNS]).starts_with("Firewall: keine filternde Firewall aktiv"));
}
