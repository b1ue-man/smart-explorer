//! FileBothDirectoryInformation (MS-FSCC 2.4.8) and CREATE-response
//! metadata as `VfsMeta`, keeping the file attributes smb2's own
//! `DirectoryEntry`/`FileInfo` drop. A reparse point is a link (never
//! descended by recursive delete and sync) unless its tag marks a data
//! reparse point: a deduplicated, compressed, cloud-placeholder or
//! tiered-storage file is an ordinary file or folder.
use crate::vfs::VfsMeta;

pub(super) const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
pub(super) const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0000_0004;
pub(super) const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
pub(super) const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

/// Fixed part of one FileBothDirectoryInformation entry: NextEntryOffset,
/// FileIndex, four times, EndOfFile, AllocationSize, FileAttributes,
/// FileNameLength, EaSize, ShortNameLength, Reserved, ShortName[24].
const FIXED_LEN: usize = 94;
const OFFSET_CREATION: usize = 8;
const OFFSET_LAST_WRITE: usize = 24;
const OFFSET_END_OF_FILE: usize = 40;
const OFFSET_ATTRIBUTES: usize = 56;
const OFFSET_NAME_LENGTH: usize = 60;
/// EaSize, which holds the reparse tag when FILE_ATTRIBUTE_REPARSE_POINT is
/// set (MS-FSCC 2.4.8).
const OFFSET_EA_SIZE_OR_TAG: usize = 64;

/// Reparse tags of data files (MS-FSCC 2.1.2.1): the entry is the file itself,
/// only stored differently. Anything else (name surrogates such as symlinks,
/// junctions and WSL links, NFS special files, DFS, unknown tags) is a link.
const IO_REPARSE_TAG_HSM: u32 = 0xC000_0004;
const IO_REPARSE_TAG_HSM2: u32 = 0x8000_0006;
const IO_REPARSE_TAG_SIS: u32 = 0x8000_0007;
const IO_REPARSE_TAG_DEDUP: u32 = 0x8000_0013;
const IO_REPARSE_TAG_WOF: u32 = 0x8000_0017;
/// IO_REPARSE_TAG_CLOUD and CLOUD_1…CLOUD_F differ only in bits 12–15.
const IO_REPARSE_TAG_CLOUD: u32 = 0x9000_001A;
const IO_REPARSE_TAG_CLOUD_MASK: u32 = 0xFFFF_0FFF;

/// 100-ns intervals between 1601-01-01 and 1970-01-01.
const EPOCH_DIFF_100NS: i128 = 116_444_736_000_000_000;

/// The metadata SMB reports for one entry (directory listing or CREATE).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Attributes {
    pub(super) attributes: u32,
    pub(super) size: u64,
    /// Windows FILETIME (100 ns since 1601), 0 = unknown.
    pub(super) last_write: u64,
    pub(super) creation: u64,
    /// The reparse tag of a reparse point; `None` when the server did not
    /// report one (such an entry stays link-like).
    pub(super) reparse_tag: Option<u32>,
}

impl Attributes {
    pub(super) fn is_dir(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    }

    pub(super) fn is_reparse_point(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    /// A redirecting reparse point: every reparse point except a data one
    /// with a known tag.
    pub(super) fn is_link(&self) -> bool {
        self.is_reparse_point() && !self.reparse_tag.is_some_and(is_data_reparse_tag)
    }
}

/// Whether a reparse tag marks data stored differently (deduplication, WOF
/// compression, cloud placeholders, HSM, single-instance storage) rather than
/// a link to another name.
pub(super) fn is_data_reparse_tag(tag: u32) -> bool {
    matches!(
        tag,
        IO_REPARSE_TAG_HSM
            | IO_REPARSE_TAG_HSM2
            | IO_REPARSE_TAG_SIS
            | IO_REPARSE_TAG_DEDUP
            | IO_REPARSE_TAG_WOF
    ) || tag & IO_REPARSE_TAG_CLOUD_MASK == IO_REPARSE_TAG_CLOUD
}

/// The ReparseTag of a FileAttributeTagInformation buffer (MS-FSCC 2.4.6:
/// FileAttributes, then ReparseTag); `None` when it is too short.
pub(super) fn attribute_tag(buffer: &[u8]) -> Option<u32> {
    u32_at(buffer, 4).ok()
}

/// Unix milliseconds of a FILETIME; 0 stays "unknown".
pub(super) fn filetime_ms(filetime: u64) -> i64 {
    if filetime == 0 {
        return 0;
    }
    let ms = (i128::from(filetime) - EPOCH_DIFF_100NS) / 10_000;
    i64::try_from(ms).unwrap_or(0)
}

pub(super) fn meta(name: String, attributes: &Attributes) -> VfsMeta {
    let is_dir = attributes.is_dir();
    VfsMeta {
        is_dir,
        is_symlink: attributes.is_link(),
        size: if is_dir { 0 } else { attributes.size },
        mtime_ms: filetime_ms(attributes.last_write),
        btime_ms: filetime_ms(attributes.creation),
        hidden: attributes.attributes & FILE_ATTRIBUTE_HIDDEN != 0 || name.starts_with('.'),
        system: attributes.attributes & FILE_ATTRIBUTE_SYSTEM != 0,
        name,
        id: None,
        content_md5: None,
    }
}

/// The server level lists only the configured share: other shares of the
/// server are neither enumerated nor browsable through this connection.
pub(super) fn server_level(share: &str) -> Vec<VfsMeta> {
    vec![VfsMeta {
        name: share.to_string(),
        is_dir: true,
        ..VfsMeta::default()
    }]
}

fn bytes<const N: usize>(entry: &[u8], at: usize) -> Result<[u8; N], String> {
    entry
        .get(at..at + N)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| "SMB-Verzeichniseintrag ist abgeschnitten".to_string())
}

fn u32_at(entry: &[u8], at: usize) -> Result<u32, String> {
    bytes::<4>(entry, at).map(u32::from_le_bytes)
}

fn u64_at(entry: &[u8], at: usize) -> Result<u64, String> {
    bytes::<8>(entry, at).map(u64::from_le_bytes)
}

/// Entries of one QUERY_DIRECTORY output buffer without `.` and `..`. Names
/// arrive in smb2's wire mapping (reserved characters in the private-use
/// area) and are decoded back to the characters they stand for.
pub(super) fn parse_directory_info(data: &[u8]) -> Result<Vec<VfsMeta>, String> {
    let mut entries = Vec::new();
    if data.is_empty() {
        return Ok(entries);
    }
    let mut offset = 0usize;
    loop {
        let entry = data
            .get(offset..)
            .filter(|entry| entry.len() >= FIXED_LEN)
            .ok_or_else(|| "SMB-Verzeichniseintrag ist abgeschnitten".to_string())?;
        let next = u32_at(entry, 0)? as usize;
        let name_len = u32_at(entry, OFFSET_NAME_LENGTH)? as usize;
        let name_end = FIXED_LEN
            .checked_add(name_len)
            .filter(|end| *end <= entry.len() && name_len.is_multiple_of(2))
            .ok_or_else(|| "SMB-Verzeichniseintrag hat eine ungültige Namenslänge".to_string())?;
        let units: Vec<u16> = entry[FIXED_LEN..name_end]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        let wire = String::from_utf16(&units)
            .map_err(|_| "SMB-Verzeichniseintrag hat einen ungültigen UTF-16-Namen".to_string())?;
        let name = smb2::decode_name(&wire).into_owned();
        if !matches!(name.as_str(), "" | "." | "..") {
            let mut attributes = Attributes {
                attributes: u32_at(entry, OFFSET_ATTRIBUTES)?,
                size: u64_at(entry, OFFSET_END_OF_FILE)?,
                last_write: u64_at(entry, OFFSET_LAST_WRITE)?,
                creation: u64_at(entry, OFFSET_CREATION)?,
                reparse_tag: None,
            };
            if attributes.is_reparse_point() {
                attributes.reparse_tag = Some(u32_at(entry, OFFSET_EA_SIZE_OR_TAG)?);
            }
            entries.push(meta(name, &attributes));
        }
        if next == 0 {
            return Ok(entries);
        }
        if next < name_end || next >= entry.len() {
            return Err("SMB-Verzeichniseintrag verweist auf eine ungültige Folgeposition".into());
        }
        offset += next;
    }
}
