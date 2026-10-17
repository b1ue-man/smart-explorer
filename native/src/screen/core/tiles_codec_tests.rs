use super::*;

/// Columns 0–63 one colour (solid tiles), 64–127 stripes (LZ4), from 128 on
/// noise (raw). Alpha is noise everywhere: the codec must ignore it.
fn picture(size: FrameSize, seed: u32) -> Vec<u8> {
    let mut state = seed.max(1);
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as u8
    };
    let mut data = vec![0u8; size.bgra_len()];
    for (index, pixel) in data.chunks_exact_mut(4).enumerate() {
        let x = index as u32 % size.width;
        let colour = match x {
            0..=63 => [10, 20, 30],
            64..=127 if (x / 8).is_multiple_of(2) => [200, 100, 50],
            64..=127 => [50, 100, 200],
            _ => [noise(), noise(), noise()],
        };
        pixel[..3].copy_from_slice(&colour);
        pixel[3] = noise();
    }
    data
}

fn opaque(data: &[u8]) -> Vec<u8> {
    data.chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 0xFF])
        .collect()
}

/// The encodings of all tiles of `records`, read from their headers.
fn encodings(records: &[Vec<u8>]) -> Vec<u8> {
    let mut found = Vec::new();
    for record in records {
        let mut at = RECORD_HEADER_BYTES;
        while at < record.len() {
            found.push(record[at + 6]);
            let len = u32::from_be_bytes(record[at + 7..at + 11].try_into().unwrap());
            at += TILE_HEADER_BYTES + len as usize;
        }
    }
    found
}

fn apply_all(decoder: &mut TileDecoder, records: &[Vec<u8>]) -> usize {
    records
        .iter()
        .map(|record| decoder.apply_record(record).unwrap())
        .sum()
}

#[test]
fn screen_task_tiles_lossless_roundtrip() {
    // 100 × 81: 2 × 2 tiles, the right and bottom ones 36 and 17 pixels.
    let size = FrameSize::new(100, 81);
    let mut encoder = TileEncoder::new(size).unwrap();
    let mut decoder = TileDecoder::new(size).unwrap();
    let first = picture(size, 1);
    let view = BgraView::new(size, 400, &first).unwrap();
    let records = encoder
        .encode(&view, false, MAX_TILES_RECORD_BYTES)
        .unwrap();
    assert_eq!(
        apply_all(&mut decoder, &records),
        4,
        "first frame is a key frame"
    );
    assert_eq!(decoder.image().data(), opaque(&first).as_slice());

    // Nothing changed: nothing to send.
    assert!(encoder
        .encode(&view, false, MAX_TILES_RECORD_BYTES)
        .unwrap()
        .is_empty());

    // One pixel of tile (1, 1) changes; an alpha change alone does not count.
    let mut second = first.clone();
    second[(70 * 100 + 90) * 4] ^= 0xFF;
    second[3] ^= 0x55;
    let view = BgraView::new(size, 400, &second).unwrap();
    let records = encoder
        .encode(&view, false, MAX_TILES_RECORD_BYTES)
        .unwrap();
    assert_eq!(apply_all(&mut decoder, &records), 1);
    assert_eq!(decoder.image().data(), opaque(&second).as_slice());

    // A key frame sends every tile again.
    let records = encoder.encode(&view, true, MAX_TILES_RECORD_BYTES).unwrap();
    assert_eq!(apply_all(&mut decoder, &records), 4);
    assert_eq!(decoder.image().data(), opaque(&second).as_slice());

    // 200 × 130 with a padded row pitch: all three encodings, and records at
    // the smallest limit split the key frame.
    let size = FrameSize::new(200, 130);
    let content = picture(size, 7);
    let stride = 200 * 4 + 64;
    let mut padded = vec![0xAA; stride * 130];
    for (row, line) in content.chunks_exact(800).enumerate() {
        padded[row * stride..row * stride + 800].copy_from_slice(line);
    }
    let view = BgraView::new(size, stride, &padded).unwrap();
    let mut encoder = TileEncoder::new(size).unwrap();
    let mut decoder = TileDecoder::new(size).unwrap();
    let records = encoder.encode(&view, true, MIN_RECORD_LIMIT).unwrap();
    assert!(records.len() > 1);
    assert!(records
        .iter()
        .all(|record| record.len() <= MIN_RECORD_LIMIT));
    assert_eq!(apply_all(&mut decoder, &records), 12);
    assert_eq!(decoder.image().data(), opaque(&content).as_slice());
    let used = encodings(&records);
    for encoding in [ENCODING_SOLID, ENCODING_RAW, ENCODING_LZ4] {
        assert!(used.contains(&encoding), "encoding {encoding} used");
    }
    let one_record = encoder.encode(&view, true, MAX_TILES_RECORD_BYTES).unwrap();
    assert_eq!(one_record.len(), 1);
    assert!(
        one_record[0].len() < content.len(),
        "solid and LZ4 tiles shrink"
    );

    // A frame of the wrong size or a record limit out of range is refused.
    let other = vec![0u8; 64 * 64 * 4];
    let small = BgraView::new(FrameSize::new(64, 64), 256, &other).unwrap();
    assert!(encoder
        .encode(&small, false, MAX_TILES_RECORD_BYTES)
        .is_err());
    assert!(encoder.encode(&view, false, MIN_RECORD_LIMIT - 1).is_err());
    assert!(encoder
        .encode(&view, false, MAX_TILES_RECORD_BYTES + 1)
        .is_err());

    // The decoder refuses records that would not rebuild the frame.
    let mut decoder = TileDecoder::new(FrameSize::new(100, 81)).unwrap();
    let solid = |x: u16, y: u16, width: u8, height: u8| {
        let mut record = vec![0, 1];
        record.extend_from_slice(&x.to_be_bytes());
        record.extend_from_slice(&y.to_be_bytes());
        record.extend_from_slice(&[width, height, ENCODING_SOLID, 0, 0, 0, 4, 1, 2, 3, 0xFF]);
        record
    };
    assert_eq!(decoder.apply_record(&solid(64, 64, 36, 17)).unwrap(), 1);
    assert!(
        decoder.apply_record(&solid(1, 0, 64, 64)).is_err(),
        "off the grid"
    );
    assert!(
        decoder.apply_record(&solid(0, 64, 64, 64)).is_err(),
        "edge tile too tall"
    );
    assert!(
        decoder.apply_record(&solid(128, 0, 64, 64)).is_err(),
        "outside"
    );
    let mut trailing = solid(0, 0, 64, 64);
    trailing.push(0);
    assert!(decoder.apply_record(&trailing).is_err());
    let mut short_raw = vec![0, 1, 0, 0, 0, 0, 64, 64, ENCODING_RAW, 0, 0, 0, 100];
    short_raw.extend_from_slice(&[0; 100]);
    assert!(decoder.apply_record(&short_raw).is_err());
    let mut broken = vec![0, 1, 0, 0, 0, 0, 64, 64, ENCODING_LZ4, 0, 0, 0, 8];
    broken.extend_from_slice(&[0xFF; 8]);
    assert!(decoder.apply_record(&broken).is_err());
    let unknown = [0, 1, 0, 0, 0, 0, 64, 64, 9, 0, 0, 0, 4, 1, 2, 3, 4];
    assert!(decoder.apply_record(&unknown).is_err());
}
