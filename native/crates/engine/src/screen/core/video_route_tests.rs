use super::*;

/// A TCP link that takes every record.
struct Link;

impl RecordSender for Link {
    fn send<'a>(&'a self, _record: &'a [u8]) -> SendFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn link(&self) -> &str {
        "punched"
    }
}

#[test]
fn udp_task_tail_names_the_last_datagram_picture_once() {
    let route = VideoRoute::default();
    assert_eq!(route.take_tail(Instant::now() + TAIL_AFTER), None);
    route.note_sent(PictureCarrier::Udp(VideoPath::Direct), 3, 4, 40);
    let sent = Instant::now();
    // Not before `TAIL_AFTER`; then once.
    assert_eq!(route.take_tail(sent), None);
    let later = sent + TAIL_AFTER;
    let tail = FrameLoss {
        gop_id: 4,
        frame_id: 40,
    };
    assert_eq!(route.take_tail(later), Some(tail));
    assert_eq!(route.take_tail(later + TAIL_AFTER), None);
    // A newer picture is named in its turn.
    route.note_sent(PictureCarrier::Udp(VideoPath::QuicDirect), 2, 4, 41);
    let later = Instant::now() + TAIL_AFTER;
    assert_eq!(
        route.take_tail(later),
        Some(FrameLoss {
            gop_id: 4,
            frame_id: 41
        })
    );
    // Pictures on the streams or the TCP link lose nothing: no tail.
    route.note_sent(PictureCarrier::Udp(VideoPath::Direct), 1, 5, 50);
    route.note_streams(false, false);
    assert_eq!(route.take_tail(later + TAIL_AFTER), None);
    route.note_sent(PictureCarrier::Tcp, 1, 6, 60);
    assert_eq!(route.take_tail(Instant::now() + TAIL_AFTER), None);
}

#[test]
fn udp_task_stats_name_the_carrier_of_the_pictures() {
    let report = |route: &VideoRoute| {
        let mut chain = ChainReport::default();
        route.report_into(&mut chain);
        chain
    };
    let route = VideoRoute::default();
    // No offer and no picture: nothing is named.
    assert_eq!(report(&route), ChainReport::default());
    route.offer(Carriers {
        datagrams: true,
        udp: true,
    });
    route.note_report(VideoReport {
        lost_pictures: 2,
        skipped_pictures: 1,
        recovered_pictures: 5,
        lost_packets: 9,
        recovered_packets: 7,
        ..VideoReport::default()
    });
    route.note_failed();
    route.note_sent(PictureCarrier::Udp(VideoPath::Direct), 4, 1, 10);
    let chain = report(&route);
    assert_eq!(chain.udp_path.as_deref(), Some("direct"));
    assert_eq!(
        chain.udp_lost_pictures,
        Some(3),
        "viewer's 2 and the host's 1"
    );
    assert_eq!(chain.udp_skipped_pictures, Some(1));
    assert_eq!(chain.udp_recovered_pictures, Some(5));
    assert_eq!(chain.udp_lost_packets, Some(9));
    assert_eq!(chain.udp_recovered_packets, Some(7));
    assert_eq!(chain.tcp_path, None);
    route.note_sent(PictureCarrier::Udp(VideoPath::Relay), 4, 1, 11);
    assert_eq!(report(&route).udp_path.as_deref(), Some("relay"));
    // „TCP“ without a link: the streams, with the reason the link failed.
    route.set_tcp_failure(Some("TCP direkt nicht möglich: x".into()));
    route.note_streams(true, true);
    let chain = report(&route);
    assert_eq!(chain.udp_path, None);
    assert_eq!(chain.tcp_path.as_deref(), Some("relay"));
    assert_eq!(
        chain.tcp_failure.as_deref(),
        Some("TCP direkt nicht möglich: x")
    );
    route.note_streams(false, true);
    assert_eq!(report(&route).tcp_path.as_deref(), Some("quic_stream"));
    // „UDP“ on the streams (an older viewer) names no TCP path.
    route.note_streams(false, false);
    assert_eq!(report(&route).tcp_path, None);
    // The link came up: the pictures take it, the reason is gone.
    route.set_record(Some(Arc::new(Link)));
    assert!(route.record_carrier().is_some());
    route.note_sent(PictureCarrier::Tcp, 1, 2, 20);
    let chain = report(&route);
    assert_eq!(chain.tcp_path.as_deref(), Some("direct"));
    assert_eq!(chain.tcp_link.as_deref(), Some("punched"));
    assert_eq!(chain.tcp_failure, None);
    // It ends: no carrier any more, and the reason says why.
    route.record_ended("Verbindung zurückgesetzt");
    assert!(route.record_carrier().is_none());
    route.note_streams(false, true);
    assert_eq!(
        report(&route).tcp_failure.as_deref(),
        Some("TCP-Verbindung beendet: Verbindung zurückgesetzt")
    );
}
