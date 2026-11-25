use super::*;

/// Deterministic bytes (xorshift), so a failure repeats.
fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

fn block(k: usize, m: usize, len: usize) -> (Vec<Vec<u8>>, Vec<Option<Vec<u8>>>) {
    let data: Vec<Vec<u8>> = (0..k).map(|j| bytes(j as u64 + 1, len)).collect();
    let refs: Vec<&[u8]> = data.iter().map(Vec::as_slice).collect();
    let parity = encode_parity(&refs, m).unwrap();
    assert_eq!(parity.len(), m);
    let shards = data.iter().cloned().chain(parity).map(Some).collect();
    (data, shards)
}

fn rebuilt(k: usize, mut shards: Vec<Option<Vec<u8>>>, lost: &[usize]) -> io::Result<Vec<Vec<u8>>> {
    for index in lost {
        shards[*index] = None;
    }
    let missing = lost.iter().filter(|index| **index < k).count();
    assert_eq!(reconstruct(k, &mut shards)?, missing);
    Ok(shards[..k]
        .iter()
        .map(|shard| shard.clone().unwrap())
        .collect())
}

#[test]
fn udp_task_fec_rebuilds_every_single_and_double_loss_within_parity() {
    // The geometries of the picture packets: 20 % parity, at least one.
    for (k, m) in [(1, 1), (2, 1), (5, 1), (6, 2), (9, 2), (20, 4), (100, 20)] {
        let (data, shards) = block(k, m, 13);
        let n = k + m;
        for a in 0..n {
            assert_eq!(
                rebuilt(k, shards.clone(), &[a]).unwrap(),
                data,
                "{k}+{m} lost {a}"
            );
            for b in a + 1..n {
                let result = rebuilt(k, shards.clone(), &[a, b]);
                if m >= 2 {
                    assert_eq!(result.unwrap(), data, "{k}+{m} lost {a},{b}");
                } else if a < k || b < k {
                    // Two losses with one parity shard: beyond the budget.
                    assert!(result.is_err(), "{k}+{m} lost {a},{b}");
                }
            }
        }
    }
}

#[test]
fn udp_task_fec_rebuilds_up_to_its_parity_and_refuses_more() {
    // Every pattern of exactly four losses of a 20 + 4 block.
    let (k, m) = (20, 4);
    let (data, shards) = block(k, m, 7);
    let n = k + m;
    for a in 0..n {
        for b in a + 1..n {
            for c in b + 1..n {
                for d in c + 1..n {
                    let lost = [a, b, c, d];
                    assert_eq!(rebuilt(k, shards.clone(), &lost).unwrap(), data, "{lost:?}");
                }
            }
        }
    }
    // A full 100 + 20 block loses its first 20 data shards and gets them back.
    let (data, shards) = block(100, 20, 31);
    let lost: Vec<usize> = (0..20).collect();
    assert_eq!(rebuilt(100, shards.clone(), &lost).unwrap(), data);
    // Five data shards lost with four parity shards cannot be rebuilt.
    let (_, shards) = block(k, m, 7);
    assert!(rebuilt(k, shards, &[0, 1, 2, 3, 4]).is_err());
    // Field: every element but zero has its inverse; zero has none.
    for a in 1..=255u8 {
        assert_eq!(mul(a, inv(a).unwrap()), 1, "{a}");
    }
    assert_eq!(inv(0), None);
    // Geometry: no empty block, at most 256 shards, shards of one length.
    assert!(encode_parity(&[], 1).is_err());
    let shard = [0u8; 4];
    let many: Vec<&[u8]> = (0..250).map(|_| &shard[..]).collect();
    assert!(encode_parity(&many, 7).is_err());
    assert!(encode_parity(&[&shard[..], &shard[..3]], 1).is_err());
}
