use super::*;

const DIR: &str = r"C:\Users\Jürgen\AppData\Local\Programs\Smart Explorer";

fn both(_: &str) -> bool {
    true
}

fn line(fields: &[&str]) -> String {
    fields.join("\t")
}

/// One `R` line as the query prints it.
fn rule_line(name: &str, protocol: &str, profile: &str, program: &str) -> String {
    line(&[
        "R", name, "True", "Inbound", "Allow", profile, protocol, program, "Any", "Any",
    ])
}

/// Every peer program gets an inbound allow rule for TCP and for UDP in
/// every profile, bound to its path; a terminal-only folder has no desktop
/// app and gets only the rules of se.exe.
#[test]
fn firewall_rules_task_windows_rule_composition() {
    let rules = expected_rules(&format!("{DIR}\\"), &both);
    let names: Vec<&str> = rules.iter().map(|rule| rule.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Smart Explorer Peer TCP (se.exe)",
            "Smart Explorer Peer UDP (se.exe)",
            "Smart Explorer Peer TCP (Smart Explorer.exe)",
            "Smart Explorer Peer UDP (Smart Explorer.exe)",
        ]
    );
    assert_eq!(rules[0].program, format!("{DIR}\\se.exe"));
    assert_eq!(rules[3].program, format!("{DIR}\\Smart Explorer.exe"));
    assert_eq!(
        rules[1].add_args(),
        [
            "advfirewall".to_string(),
            "firewall".into(),
            "add".into(),
            "rule".into(),
            "name=Smart Explorer Peer UDP (se.exe)".into(),
            "dir=in".into(),
            "action=allow".into(),
            format!("program={DIR}\\se.exe"),
            "enable=yes".into(),
            "profile=any".into(),
            "protocol=UDP".into(),
            format!("description={DESCRIPTION}"),
        ]
    );

    // Case: a canonicalized folder (`\\?\`) names the path processes run from.
    let verbatim = expected_rules(&format!(r"\\?\{DIR}"), &both);
    assert_eq!(verbatim[0].program, rules[0].program);
    assert_eq!(plain_path(r"\\?\UNC\server\share\se"), r"\\server\share\se");

    // Case: a terminal-only installation.
    let cli_only = expected_rules(DIR, &|file| file == "se.exe");
    assert_eq!(cli_only.len(), 2);
    assert!(cli_only
        .iter()
        .all(|rule| rule.program.ends_with(r"\se.exe")));

    // Case: deletes name the program; only adds count as failures; a quote
    // in a path is doubled for PowerShell.
    let delete = Step::DeleteFor {
        name: rules[0].name.clone(),
        program: r"C:\O'Brien\se.exe".into(),
    };
    assert_eq!(
        delete.args()[4..],
        [
            "name=Smart Explorer Peer TCP (se.exe)".to_string(),
            r"program=C:\O'Brien\se.exe".into()
        ]
    );
    let script = elevated_script(
        r"C:\Windows\System32\netsh.exe",
        &[delete, Step::Add(rules[0].clone())],
    );
    assert!(
        script.starts_with("$PSModuleAutoLoadingPreference='None';"),
        "{script}"
    );
    assert!(script.contains(r"'program=C:\O''Brien\se.exe'"), "{script}");
    assert_eq!(script.matches("$LASTEXITCODE").count(), 1, "{script}");
    assert!(script.ends_with("exit $f"), "{script}");
}

/// The query output becomes rules and profiles; the plan keeps what fits,
/// repairs an outdated rule of this program (protocol, profiles, switched
/// off, blocking), leaves another installation's rule and replaces the
/// legacy rule; the legacy rule alone raises no confirmation.
#[test]
fn firewall_rules_task_windows_query_parse_and_repair() {
    let se = format!("{DIR}\\se.exe");
    let app = format!("{DIR}\\Smart Explorer.exe");
    let other = r"D:\Portable\Smart Explorer\se.exe";
    let text = [
        rule_line(LEGACY_RULE, "UDP", "Domain, Private", &app),
        // Fits, though Windows reports the path in other case and slashes.
        rule_line(
            "Smart Explorer Peer UDP (se.exe)",
            "UDP",
            "Any",
            &se.to_uppercase().replace('\\', "/"),
        ),
        rule_line(
            "Smart Explorer Peer TCP (se.exe)",
            "TCP",
            "Domain, Private",
            &se,
        ),
        rule_line("Smart Explorer Peer TCP (se.exe)", "TCP", "Any", other),
        rule_line(
            "Smart Explorer Peer UDP (Smart Explorer.exe)",
            "TCP",
            "Any",
            &app,
        ),
        line(&["P", "Public", "True", "NotConfigured", "NotConfigured"]),
        "noise without tabs".into(),
    ]
    .join("\r\n");
    let (observed, profiles) = parse_query(&text);
    assert_eq!(observed.len(), 5);
    assert_eq!(profiles.len(), 1);
    assert!(profiles[0].inbound_rules_apply && profiles[0].local_rules_apply);
    assert_eq!(
        observed[2].profiles,
        Profiles {
            domain: true,
            private: true,
            public: false
        }
    );

    let expected = expected_rules(DIR, &both);
    let wanted = plan(&expected, &observed);
    let tcp_se = expected[0].clone();
    let tcp_app = expected[2].clone();
    let udp_app = expected[3].clone();
    assert_eq!(
        wanted.steps,
        [
            Step::DeleteName(LEGACY_RULE.into()),
            Step::DeleteFor {
                name: tcp_se.name.clone(),
                program: se.clone()
            },
            Step::Add(tcp_se),
            Step::Add(tcp_app),
            Step::DeleteFor {
                name: udp_app.name.clone(),
                program: app.clone()
            },
            Step::Add(udp_app),
        ]
    );
    assert_eq!(
        wanted.findings[0],
        format!("veraltete Regel „{LEGACY_RULE}“ (UDP für {app} in Domäne, Privat) wird ersetzt")
    );
    assert!(
        wanted.findings[1].ends_with("nicht in allen Netzprofilen"),
        "{:?}",
        wanted.findings
    );
    assert!(
        wanted.findings[2].ends_with("fehlt"),
        "{:?}",
        wanted.findings
    );
    assert!(
        wanted.findings[3].ends_with("anderes Protokoll"),
        "{:?}",
        wanted.findings
    );

    // Case: a switched-off and a blocking rule of this program are outdated.
    let mut off = observed[1].clone();
    off.enabled = false;
    assert_eq!(defect(&off, &expected[1]), Some("ausgeschaltet"));
    let mut block = observed[1].clone();
    block.allow = false;
    assert_eq!(
        defect(&block, &expected[1]),
        Some("blockiert statt zuzulassen")
    );
    let mut narrowed = observed[1].clone();
    narrowed.remote_address = "LocalSubnet".into();
    assert_eq!(
        defect(&narrowed, &expected[1]),
        Some("auf Ports oder Adressen beschränkt")
    );

    // Case: every rule fits; the legacy rule stays without a prompt.
    let fitting: Vec<Observed> = expected
        .iter()
        .map(|rule| {
            parse_query(&rule_line(
                &rule.name,
                rule.protocol.netsh(),
                "Any",
                &rule.program,
            ))
            .0[0]
                .clone()
        })
        .chain([observed[0].clone()])
        .collect();
    assert!(plan(&expected, &fitting).is_empty());
}

/// The status names every profile; a profile whose own setting blocks all
/// incoming connections, or whose group policy ignores local rules, is named
/// as a caveat; a switched-off profile is not.
#[test]
fn firewall_rules_task_windows_status_names_profiles() {
    let expected = expected_rules(DIR, &both);
    let profiles = parse_query(
        &[
            line(&["P", "Domain", "True", "True", "False"]),
            line(&["P", "Private", "False", "False", "False"]),
            line(&["P", "Public", "True", "False", "True"]),
        ]
        .join("\n"),
    )
    .1;
    let status = status_line(&expected, &profiles);
    assert!(
        status.starts_with(
            "Firewall-Regeln aktiv: eingehend TCP und UDP für se.exe und Smart Explorer.exe in allen Netzprofilen (Domäne, Privat, Öffentlich)"
        ),
        "{status}"
    );
    assert!(
        status.contains("im Profil Öffentlich ist „Alle eingehenden Verbindungen blockieren“"),
        "{status}"
    );
    assert!(
        status.contains("im Profil Domäne lässt eine Gruppenrichtlinie keine lokalen Regeln zu"),
        "{status}"
    );
    assert!(!status.contains("Profil Privat"), "{status}");
    assert!(Profiles::parse("Any").all());
    assert!(!Profiles::parse("Private").all());
    assert!(same_program(r"c:/x/SE.EXE", r"C:\x\se.exe"));
}
