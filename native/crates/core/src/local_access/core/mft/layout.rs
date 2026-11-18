//! Volume geometry and the `$MFT` extents, decoded from the raw output of
//! `FSCTL_GET_NTFS_VOLUME_DATA` and `FSCTL_GET_RETRIEVAL_POINTERS`, and the
//! aligned read plan over those extents.
use super::bytes::{i64_at, u32_at, u64_at};
use std::io;

/// Size of `NTFS_VOLUME_DATA_BUFFER`.
pub const VOLUME_DATA_LEN: usize = 96;
/// Largest record segment accepted (NTFS uses 1 KiB or 4 KiB).
const MAX_RECORD: u32 = 64 * 1024;

fn invalid(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text.to_string())
}

/// The geometry fields of `NTFS_VOLUME_DATA_BUFFER` the reader uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NtfsVolumeData {
    pub serial: u64,
    pub bytes_per_sector: u32,
    pub bytes_per_cluster: u32,
    pub bytes_per_record: u32,
    /// Initialised length of `$MFT`; bytes beyond are not records.
    pub mft_valid_length: u64,
}

impl NtfsVolumeData {
    pub fn record_count(&self) -> u64 {
        self.mft_valid_length / u64::from(self.bytes_per_record)
    }
}

pub fn parse_volume_data(bytes: &[u8]) -> io::Result<NtfsVolumeData> {
    let field = |at| u32_at(bytes, at).ok_or_else(|| invalid("NTFS-Volumedaten zu kurz"));
    let data = NtfsVolumeData {
        serial: u64_at(bytes, 0).ok_or_else(|| invalid("NTFS-Volumedaten zu kurz"))?,
        bytes_per_sector: field(40)?,
        bytes_per_cluster: field(44)?,
        bytes_per_record: field(48)?,
        mft_valid_length: u64::try_from(
            i64_at(bytes, 56).ok_or_else(|| invalid("NTFS-Volumedaten zu kurz"))?,
        )
        .map_err(|_| invalid("negative MFT-Länge"))?,
    };
    let sector = data.bytes_per_sector;
    if !sector.is_power_of_two() || !(512..=MAX_RECORD).contains(&sector) {
        return Err(invalid("ungültige Sektorgröße"));
    }
    if data.bytes_per_cluster == 0 || !data.bytes_per_cluster.is_multiple_of(sector) {
        return Err(invalid("ungültige Clustergröße"));
    }
    let record = data.bytes_per_record;
    if !record.is_power_of_two() || !(512..=MAX_RECORD).contains(&record) {
        return Err(invalid("ungültige MFT-Datensatzgröße"));
    }
    if data.mft_valid_length < u64::from(record) {
        return Err(invalid("leere MFT"));
    }
    Ok(data)
}

/// One `$MFT` extent: `clusters` clusters from virtual cluster `vcn`, stored
/// at logical cluster `lcn` (`None`: a sparse hole without clusters).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MftExtent {
    pub vcn: u64,
    pub lcn: Option<u64>,
    pub clusters: u64,
}

/// Decode a `RETRIEVAL_POINTERS_BUFFER`. Returns the extents and the VCN
/// after the last one (the start of a follow-up query on `ERROR_MORE_DATA`).
pub fn parse_retrieval_pointers(bytes: &[u8]) -> io::Result<(Vec<MftExtent>, u64)> {
    let short = || invalid("Extent-Liste der MFT zu kurz");
    let count = u32_at(bytes, 0).ok_or_else(short)? as usize;
    let mut vcn = u64::try_from(i64_at(bytes, 8).ok_or_else(short)?)
        .map_err(|_| invalid("negative Start-VCN der MFT"))?;
    let mut extents = Vec::with_capacity(count.min(bytes.len() / 16));
    for index in 0..count {
        let at = 16 + index * 16;
        let next = i64_at(bytes, at).ok_or_else(short)?;
        let lcn = i64_at(bytes, at + 8).ok_or_else(short)?;
        let next = u64::try_from(next).map_err(|_| invalid("negative VCN der MFT"))?;
        if next <= vcn {
            return Err(invalid("MFT-Extents nicht aufsteigend"));
        }
        extents.push(MftExtent {
            vcn,
            lcn: u64::try_from(lcn).ok(),
            clusters: next - vcn,
        });
        vcn = next;
    }
    Ok((extents, vcn))
}

/// One read: `len` bytes at MFT byte offset `logical`, from volume byte
/// offset `disk` (`None` for a sparse hole that is not read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadSpan {
    pub logical: u64,
    pub disk: Option<u64>,
    pub len: u64,
}

/// Split the extents into reads of at most `chunk` bytes, in MFT order,
/// up to the valid MFT length rounded up to whole sectors (unbuffered
/// reads need sector multiples). `chunk` must be a sector multiple.
pub fn plan_reads(
    data: &NtfsVolumeData,
    extents: &[MftExtent],
    chunk: u64,
) -> io::Result<Vec<ReadSpan>> {
    let sector = u64::from(data.bytes_per_sector);
    let cluster = u64::from(data.bytes_per_cluster);
    if chunk == 0 || !chunk.is_multiple_of(sector) {
        return Err(invalid("Leseblock ist kein Sektorvielfaches"));
    }
    let limit = data.mft_valid_length.div_ceil(sector) * sector;
    let overflow = || invalid("MFT-Extent außerhalb des Adressraums");
    let mut spans = Vec::new();
    let mut expected = 0u64;
    for extent in extents {
        let start = extent.vcn.checked_mul(cluster).ok_or_else(overflow)?;
        if start != expected {
            return Err(invalid("MFT-Extents nicht lückenlos"));
        }
        let length = extent.clusters.checked_mul(cluster).ok_or_else(overflow)?;
        expected = start.checked_add(length).ok_or_else(overflow)?;
        let end = expected.min(limit);
        let mut logical = start;
        while logical < end {
            let len = chunk.min(end - logical);
            let disk = match extent.lcn {
                Some(lcn) => Some(
                    lcn.checked_mul(cluster)
                        .and_then(|base| base.checked_add(logical - start))
                        .ok_or_else(overflow)?,
                ),
                None => None,
            };
            spans.push(ReadSpan { logical, disk, len });
            logical += len;
        }
        if expected >= limit {
            return Ok(spans);
        }
    }
    Err(invalid("MFT-Extents decken die gültige MFT-Länge nicht ab"))
}
