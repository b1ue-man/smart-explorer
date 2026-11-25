//! Erasure code of the picture datagrams (`video_transport.rs`): Moonlight
//! and Sunshine protect each video frame with Reed-Solomon parity packets,
//! by default 20 % (`fec_percentage`, plan 2026-11-14 `bildkette.md` 2.1),
//! so a lost packet is rebuilt on the viewer instead of being sent again.
//!
//! Systematic Reed-Solomon over GF(2^8) (polynomial x^8 + x^4 + x^3 + x^2 + 1,
//! `0x11D`, generator 2): the `k` data shards go unchanged, parity shard `i`
//! is `sum_j c(i, j) * data_j` with the Cauchy matrix `c(i, j) = 1 / (x_i +
//! y_j)`, `x_i = k + i`, `y_j = j`. All `x_i` and `y_j` are distinct elements
//! of the field while `k + m <= 256`, and every square submatrix of a Cauchy
//! matrix is invertible, so any `k` of the `k + m` shards give the data back
//! (the code is MDS). Recovery solves only for the missing data shards: with
//! `e` of them lost, `e` received parity rows minus the known data form an
//! `e x e` Cauchy system, inverted by Gauss-Jordan elimination.
use std::io;
use std::sync::OnceLock;

use super::wire::invalid;

/// Shards of one block at most: the Cauchy points `0..k+m` are distinct
/// field elements.
pub const MAX_SHARDS: usize = 256;

const fn tables() -> ([u8; 512], [u8; 256]) {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11D;
        }
        i += 1;
    }
    let mut j = 255;
    while j < 512 {
        exp[j] = exp[j - 255];
        j += 1;
    }
    (exp, log)
}

const TABLES: ([u8; 512], [u8; 256]) = tables();
const EXP: [u8; 512] = TABLES.0;
const LOG: [u8; 256] = TABLES.1;

/// Product in GF(256).
pub fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    EXP[LOG[a as usize] as usize + LOG[b as usize] as usize]
}

/// Inverse in GF(256); `None` for zero.
pub fn inv(a: u8) -> Option<u8> {
    (a != 0).then(|| EXP[255 - LOG[a as usize] as usize])
}

/// One row of products per coefficient (64 KiB, built once): the inner
/// loops look one byte up instead of two logarithms.
fn products() -> &'static [[u8; 256]] {
    static TABLE: OnceLock<Vec<[u8; 256]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..256usize)
            .map(|a| {
                let mut row = [0u8; 256];
                for (b, out) in row.iter_mut().enumerate() {
                    *out = mul(a as u8, b as u8);
                }
                row
            })
            .collect()
    })
}

/// `dst ^= c * src`, byte by byte.
fn mul_add(dst: &mut [u8], src: &[u8], c: u8) {
    match c {
        0 => {}
        1 => dst.iter_mut().zip(src).for_each(|(d, s)| *d ^= *s),
        c => {
            let row = &products()[c as usize];
            dst.iter_mut()
                .zip(src)
                .for_each(|(d, s)| *d ^= row[*s as usize]);
        }
    }
}

/// The coefficient of data shard `j` in parity shard `i` of a block with
/// `k` data shards.
fn cauchy(k: usize, i: usize, j: usize) -> u8 {
    // x_i = k + i > j = y_j, both below 256: never zero.
    inv(((k + i) as u8) ^ (j as u8)).unwrap_or(0)
}

fn check_geometry(k: usize, m: usize) -> io::Result<()> {
    if k == 0 || k + m > MAX_SHARDS {
        return Err(invalid("erasure block needs 1 to 256 shards"));
    }
    Ok(())
}

/// The `parity` parity shards of `data`; every data shard has the same
/// length.
pub fn encode_parity(data: &[&[u8]], parity: usize) -> io::Result<Vec<Vec<u8>>> {
    let k = data.len();
    check_geometry(k, parity)?;
    let len = data[0].len();
    if data.iter().any(|shard| shard.len() != len) {
        return Err(invalid("erasure shards differ in length"));
    }
    let mut out = vec![vec![0u8; len]; parity];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, shard) in data.iter().enumerate() {
            mul_add(row, shard, cauchy(k, i, j));
        }
    }
    Ok(out)
}

/// Rebuilds the missing data shards of a block with `k` data shards; the
/// first `k` entries of `shards` are the data, the rest the parity, `None`
/// where a shard was lost. Returns how many data shards it rebuilt; fewer
/// than `k` shards present is an error. Lost parity shards stay `None`.
pub fn reconstruct(k: usize, shards: &mut [Option<Vec<u8>>]) -> io::Result<usize> {
    let m = shards.len().saturating_sub(k);
    check_geometry(k, m)?;
    let missing: Vec<usize> = (0..k).filter(|j| shards[*j].is_none()).collect();
    if missing.is_empty() {
        return Ok(0);
    }
    let parity: Vec<usize> = (0..m)
        .filter(|i| shards[k + i].is_some())
        .take(missing.len())
        .collect();
    if parity.len() < missing.len() {
        return Err(invalid("too few shards to rebuild the block"));
    }
    let len = shards
        .iter()
        .flatten()
        .map(Vec::len)
        .next()
        .ok_or_else(|| invalid("erasure block without shards"))?;
    if shards.iter().flatten().any(|shard| shard.len() != len) {
        return Err(invalid("erasure shards differ in length"));
    }
    // Right-hand sides: each parity row minus the data shards present.
    let mut rhs: Vec<Vec<u8>> = Vec::with_capacity(parity.len());
    for &i in &parity {
        let mut row = shards[k + i].clone().unwrap_or_default();
        for (j, shard) in shards[..k].iter().enumerate() {
            if let Some(shard) = shard {
                mul_add(&mut row, shard, cauchy(k, i, j));
            }
        }
        rhs.push(row);
    }
    let matrix: Vec<Vec<u8>> = parity
        .iter()
        .map(|&i| missing.iter().map(|&j| cauchy(k, i, j)).collect())
        .collect();
    let inverse = invert(matrix)?;
    for (row, &j) in inverse.iter().zip(&missing) {
        let mut shard = vec![0u8; len];
        for (c, value) in row.iter().zip(&rhs) {
            mul_add(&mut shard, value, *c);
        }
        shards[j] = Some(shard);
    }
    Ok(missing.len())
}

/// Gauss-Jordan inversion of a square matrix over GF(256).
fn invert(mut a: Vec<Vec<u8>>) -> io::Result<Vec<Vec<u8>>> {
    let n = a.len();
    let mut b: Vec<Vec<u8>> = (0..n)
        .map(|r| (0..n).map(|c| u8::from(r == c)).collect())
        .collect();
    for col in 0..n {
        let pivot = (col..n)
            .find(|r| a[*r][col] != 0)
            .ok_or_else(|| invalid("erasure matrix is singular"))?;
        a.swap(col, pivot);
        b.swap(col, pivot);
        let scale = inv(a[col][col]).ok_or_else(|| invalid("erasure matrix is singular"))?;
        a[col]
            .iter_mut()
            .for_each(|value| *value = mul(*value, scale));
        b[col]
            .iter_mut()
            .for_each(|value| *value = mul(*value, scale));
        let (pivot_a, pivot_b) = (a[col].clone(), b[col].clone());
        for (r, (row_a, row_b)) in a.iter_mut().zip(b.iter_mut()).enumerate() {
            let factor = row_a[col];
            if r == col || factor == 0 {
                continue;
            }
            for (value, pivot) in row_a.iter_mut().zip(&pivot_a) {
                *value ^= mul(factor, *pivot);
            }
            for (value, pivot) in row_b.iter_mut().zip(&pivot_b) {
                *value ^= mul(factor, *pivot);
            }
        }
    }
    Ok(b)
}

#[cfg(test)]
#[path = "fec_tests.rs"]
mod tests;
