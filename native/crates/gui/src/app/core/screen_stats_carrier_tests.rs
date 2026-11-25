use super::*;

fn stats(path: PathKind, chain: Option<ChainReport>) -> HostStats {
    HostStats {
        path,
        chain,
        ..HostStats::default()
    }
}

fn udp(path: &str) -> ChainReport {
    ChainReport {
        udp_path: Some(path.into()),
        udp_lost_pictures: Some(3),
        udp_skipped_pictures: Some(2),
        udp_recovered_pictures: Some(11),
        udp_lost_packets: Some(40),
        udp_recovered_packets: Some(37),
        ..ChainReport::default()
    }
}

#[test]
fn udp_task_desktop_names_the_way_of_the_pictures() {
    // „UDP“: the direct pair, QUIC datagrams on a direct path, the relay.
    assert_eq!(
        carrier_lines(&stats(PathKind::Direct, Some(udp("direct")))),
        vec![
            "Bildweg UDP direkt",
            "Verworfen: unvollständig (UDP) 3",
            "Verworfen: Kette unterbrochen (UDP) 2",
            "Wiederhergestellt (UDP) 11 Bilder, 37 von 40 verlorenen Paketen",
        ]
    );
    assert_eq!(
        path_line(&stats(PathKind::Direct, Some(udp("quic_direct")))),
        "Bildweg UDP über QUIC (direkt)"
    );
    assert_eq!(
        path_line(&stats(PathKind::Relay, Some(udp("relay")))),
        "Bildweg über Relay (QUIC-Datagramme, kein UDP)"
    );
    // „TCP“: the direct link with how it runs, or the streams with the
    // reason the link is missing.
    let tcp = |path: &str, link: Option<&str>, failure: Option<&str>| ChainReport {
        tcp_path: Some(path.into()),
        tcp_link: link.map(str::to_string),
        tcp_failure: failure.map(str::to_string),
        ..ChainReport::default()
    };
    assert_eq!(
        carrier_lines(&stats(
            PathKind::Direct,
            Some(tcp("direct", Some("punched"), None))
        )),
        vec!["Bildweg TCP direkt (durch NAT gelocht)"]
    );
    let failure = "TCP direkt nicht möglich: Zeitüberschreitung beim Aufbau der TCP-Verbindung";
    assert_eq!(
        carrier_lines(&stats(
            PathKind::Relay,
            Some(tcp("relay", None, Some(failure)))
        )),
        vec!["Bildweg über Relay (QUIC-Strom)", failure]
    );
    assert_eq!(
        path_line(&stats(
            PathKind::Direct,
            Some(tcp("quic_stream", None, None))
        )),
        "Bildweg QUIC-Strom (direkt)"
    );
    // A host before the carriers: the QUIC streams of the connection's path.
    assert_eq!(
        carrier_lines(&stats(PathKind::Relay, None)),
        vec!["Bildweg über Relay (QUIC-Strom)"]
    );
    assert_eq!(
        path_line(&stats(PathKind::Direct, Some(ChainReport::default()))),
        "Bildweg QUIC-Strom (direkt)"
    );
}
