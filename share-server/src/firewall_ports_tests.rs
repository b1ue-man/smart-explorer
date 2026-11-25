use super::*;

fn addr(text: &str) -> SocketAddr {
    text.parse().expect("socket address")
}

fn labels_of(ports: &[Listener]) -> Vec<String> {
    ports.iter().map(Listener::label).collect()
}

/// The server admits exactly the ports the internet reaches: signaling, the
/// relay, QAD (with TLS and a relay, `SE_IROH_QAD_DISABLE`/`_BIND` as
/// `relay_qad::plan` reads them), the TCP reflector and the STUN responder;
/// a loopback listener stays out and a port is listed once.
#[test]
fn firewall_ports_task_server_port_list() {
    let all = collect(
        addr("0.0.0.0:51820"),
        Some(addr("0.0.0.0:51821")),
        qad_bind(addr("0.0.0.0:51821"), true, None, None),
        &[
            Listener::tcp(addr("0.0.0.0:51823"), "TCP-Reflektor"),
            Listener::udp(addr("[::]:3478"), "STUN"),
            Listener::tcp(addr("[::]:51820"), "doppelt"),
        ],
    );
    assert_eq!(
        labels_of(&all),
        [
            "51820/tcp",
            "51821/tcp",
            "7842/udp",
            "51823/tcp",
            "3478/udp"
        ]
    );
    assert_eq!(all[2].purpose, "QUIC-Adresserkennung (QAD)");

    // Case: behind a reverse proxy only an explicit QAD bind is reachable.
    let proxied = collect(
        addr("127.0.0.1:51820"),
        Some(addr("127.0.0.1:51821")),
        qad_bind(addr("127.0.0.1:51821"), true, None, Some(" [::]:7842 ")),
        &[],
    );
    assert_eq!(labels_of(&proxied), ["7842/udp"]);
    assert_eq!(
        qad_bind(addr("127.0.0.1:51821"), true, None, None),
        Some(addr("127.0.0.1:7842"))
    );

    // Case: no QAD without TLS, when disabled or with an invalid bind.
    assert_eq!(qad_bind(addr("0.0.0.0:51821"), false, None, None), None);
    assert_eq!(
        qad_bind(addr("0.0.0.0:51821"), true, Some("TRUE"), None),
        None
    );
    assert_eq!(qad_bind(addr("0.0.0.0:51821"), true, Some("1"), None), None);
    assert_eq!(
        qad_bind(addr("0.0.0.0:51821"), true, None, Some("nope")),
        None
    );
    let no_relay = collect(addr("0.0.0.0:51820"), None, None, &[]);
    assert_eq!(labels_of(&no_relay), ["51820/tcp"]);

    // Case: the address services follow `address_services::plan`.
    let signal = addr("0.0.0.0:51820");
    assert_eq!(
        service_bind(None, signal, 51823),
        Some(addr("0.0.0.0:51823"))
    );
    assert_eq!(
        service_bind(Some("  "), signal, 3478),
        Some(addr("0.0.0.0:3478"))
    );
    assert_eq!(service_bind(Some("OFF"), signal, 3478), None);
    assert_eq!(
        service_bind(Some("[::]:3479"), signal, 3478),
        Some(addr("[::]:3479"))
    );
    assert_eq!(service_bind(Some("nope"), signal, 3478), None);
    assert_eq!(
        service_bind(None, addr("127.0.0.1:51820"), 51823),
        Some(addr("127.0.0.1:51823"))
    );
}

/// Root runs a firewall command directly; any other account through sudo,
/// which asks in a terminal and stays quiet (`-n`) without one. The
/// effective user ID is the second `Uid:` value.
#[test]
fn firewall_ports_task_server_privilege_path() {
    let args = vec!["allow".to_string(), "51820/tcp".to_string()];
    assert_eq!(
        Privilege::Root.argv("ufw", &args),
        ["ufw", "allow", "51820/tcp"]
    );
    assert_eq!(
        Privilege::SudoAsk.argv("ufw", &args),
        ["sudo", "ufw", "allow", "51820/tcp"]
    );
    assert_eq!(
        Privilege::SudoQuiet.argv("ufw", &args),
        ["sudo", "-n", "ufw", "allow", "51820/tcp"]
    );
    assert_eq!(effective_uid("Name:\tse\nUid:\t1000\t0\t0\t0\n"), Some(0));
    assert_eq!(effective_uid("Uid:\t1000\t1000\t1000\t1000\n"), Some(1000));
    assert_eq!(effective_uid("Name:\tse\n"), None);
}

/// Windows: one rule per protocol for this program in every profile; an
/// existing rule fits only enabled, inbound, allowing, in every profile, for
/// this protocol and program.
#[test]
fn firewall_ports_task_server_windows_rules() {
    let program = plain(r"\\?\C:\Server\se-share-server.exe");
    assert_eq!(program, r"C:\Server\se-share-server.exe");
    let steps = netsh_steps(&program, &[Proto::Tcp, Proto::Udp]);
    assert_eq!(steps.len(), 4);
    assert_eq!(
        steps[0],
        [
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            "name=se-share-server TCP",
            r"program=C:\Server\se-share-server.exe"
        ]
    );
    assert_eq!(steps[3][2], "add");
    assert!(steps[3].contains(&"protocol=UDP".to_string()));
    assert!(steps[3].contains(&"profile=any".to_string()));

    let rule = |name: &str, profile: &str, protocol: &str, path: &str| {
        [
            "se-share-server ".to_string() + name,
            "True".into(),
            "Inbound".into(),
            "Allow".into(),
            profile.into(),
            protocol.into(),
            path.into(),
        ]
        .join("\t")
    };
    let both = [
        rule("TCP", "Any", "TCP", r"c:\server\SE-SHARE-SERVER.EXE"),
        rule("UDP", "Any", "UDP", &program),
    ]
    .join("\r\n");
    assert!(fits(&both, &program, &[Proto::Tcp, Proto::Udp]));
    let private_only = rule("UDP", "Private", "UDP", &program);
    assert!(!fits(&private_only, &program, &[Proto::Udp]));
    let elsewhere = rule("UDP", "Any", "UDP", r"D:\old\se-share-server.exe");
    assert!(!fits(&elsewhere, &program, &[Proto::Udp]));
    assert!(fits(&both, &program, &[Proto::Udp, Proto::Tcp]));
    let tcp_only = rule("TCP", "Any", "TCP", &program);
    assert!(!fits(&tcp_only, &program, &[Proto::Tcp, Proto::Udp]));
}

/// Linux: ufw and firewalld detection, the nftables input chains that drop
/// what no rule accepts (also behind a set block), legacy iptables filtering
/// and the commands that admit a port with the server's mark.
#[cfg(target_os = "linux")]
#[test]
fn firewall_ports_task_server_linux_detection() {
    use super::platform::*;
    assert!(ufw_active("Status: active\n"));
    assert!(!ufw_active("Status: inactive\n"));
    assert!(ufw_enabled("ENABLED=yes\n"));
    assert!(!ufw_enabled("#ENABLED=yes\nENABLED=no\n"));
    assert_eq!(
        firewalld_zones("public\n  interfaces: eth0\n", "public"),
        ["public"]
    );
    let ruleset = "table inet filter {\n\tset s {\n\t\ttype ipv4_addr\n\t}\n\tchain input {\n\t\ttype filter hook input priority filter; policy drop;\n\t\tct state established,related accept\n\t}\n}\ntable ip open {\n\tchain INPUT {\n\t\ttype filter hook input priority 0; policy accept;\n\t\tcounter packets 1 bytes 2 drop\n\t}\n\tchain FORWARD {\n\t\ttype filter hook forward priority 0; policy drop;\n\t}\n}\n";
    let chains = nft_filtering_chains(ruleset);
    let names: Vec<(&str, &str, &str)> = chains
        .iter()
        .map(|(family, table, chain, _)| (family.as_str(), table.as_str(), chain.as_str()))
        .collect();
    assert_eq!(
        names,
        [("inet", "filter", "input"), ("ip", "open", "INPUT")]
    );
    let port = Listener::udp(addr("0.0.0.0:7842"), "QAD");
    assert_eq!(
        nft_insert("inet", "filter", "input", &port),
        [
            "insert",
            "rule",
            "inet",
            "filter",
            "input",
            "udp",
            "dport",
            "7842",
            "accept",
            "comment",
            "\"se-share-server\""
        ]
    );
    assert!(iptables_filters("-P INPUT DROP\n"));
    // ufw active, but this family's INPUT never reaches ufw's chains.
    assert!(bypasses_ufw(
        "-P INPUT DROP\n-A INPUT -i lo -j ACCEPT\n-A INPUT -p tcp -m tcp --dport 51820 -j ACCEPT\n"
    ));
    assert!(!bypasses_ufw(
        "-P INPUT DROP\n-A INPUT -j ufw6-before-logging-input\n-A INPUT -j ufw6-before-input\n"
    ));
    assert!(!bypasses_ufw("-P INPUT ACCEPT\n"));
    assert!(!iptables_filters(
        "-P INPUT ACCEPT\n-A INPUT -p tcp --dport 22 -j DROP\n"
    ));
    assert_eq!(
        iptables_args(true, &port),
        [
            "-C",
            "INPUT",
            "-p",
            "udp",
            "--dport",
            "7842",
            "-m",
            "comment",
            "--comment",
            "se-share-server",
            "-j",
            "ACCEPT"
        ]
    );
}
