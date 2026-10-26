use super::*;

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn video_header() -> GopHeader {
    GopHeader {
            family_layout_revision: None,
        frame_contract: None,
        gop_id: 0x0102_0304_0506_0708,
        kind: StreamKind::Video,
        codec: CodecKind::H264,
        profile: Some(H264Profile::High),
        size: FrameSize::new(1920, 1080),
    }
}

fn tiles_header() -> GopHeader {
    GopHeader {
            family_layout_revision: None,
        frame_contract: None,
        gop_id: 9,
        kind: StreamKind::Preview,
        codec: CodecKind::Tiles,
        profile: None,
        size: FrameSize::new(320, 180),
    }
}

fn record(frame_id: u64, flags: u8, payload: Vec<u8>) -> FrameRecord {
    FrameRecord {
        frame_id,
        capture_ts_us: 1_000,
        flags,
        payload,
    }
}

#[test]
fn screen_task_wire_picture_stream_roundtrip() {
    run(async {
        // Picture stream: head, two records, clean end.
        let header = video_header();
        let records = [
            record(7, FLAG_KEYFRAME, vec![0, 0, 0, 1, 0x65]),
            record(8, 0, vec![0, 0, 0, 1, 0x41]),
        ];
        let mut stream = Vec::new();
        write_gop_header(&mut stream, &header).await.unwrap();
        for entry in &records {
            write_frame_record(&mut stream, &header, entry)
                .await
                .unwrap();
        }
        assert_eq!(
            &stream[20..20 + RECORD_HEADER_BYTES],
            &[0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0x03, 0xE8, 1]
        );
        let mut reader = stream.as_slice();
        let read_header = read_gop_header(&mut reader).await.unwrap();
        assert_eq!(read_header, header);
        let mut sequence = RecordSequence::default();
        for entry in &records {
            let read = read_frame_record(&mut reader, &read_header)
                .await
                .unwrap()
                .unwrap();
            sequence.accept(&read_header, &read).unwrap();
            assert_eq!(&read, entry);
        }
        assert_eq!(
            read_frame_record(&mut reader, &read_header).await.unwrap(),
            None
        );

        // Record order: key frame first, rising ids, tile frames over several records.
        let tiles = tiles_header();
        let mut sequence = RecordSequence::default();
        sequence
            .accept(&tiles, &record(1, FLAG_KEYFRAME | FLAG_MORE, vec![]))
            .unwrap();
        assert!(sequence.is_open());
        assert!(sequence.accept(&tiles, &record(2, 0, vec![])).is_err());
        sequence
            .accept(&tiles, &record(1, FLAG_KEYFRAME, vec![]))
            .unwrap();
        assert!(!sequence.is_open());
        assert!(sequence.accept(&tiles, &record(1, 0, vec![])).is_err());
        sequence.accept(&tiles, &record(2, 0, vec![])).unwrap();
        assert!(RecordSequence::default()
            .accept(&tiles, &record(1, 0, vec![]))
            .is_err());
        assert!(RecordSequence::default()
            .accept(&header, &record(1, FLAG_KEYFRAME | FLAG_MORE, vec![]))
            .is_err());
        assert!(RecordSequence::default()
            .accept(&tiles, &record(1, 0x80, vec![]))
            .is_err());
    });
}
