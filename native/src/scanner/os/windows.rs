#[cfg(windows)]
pub(super) fn get_attrs(meta: &std::fs::Metadata) -> (bool, bool) {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;

    let a = meta.file_attributes();
    (
        a & FILE_ATTRIBUTE_HIDDEN != 0,
        a & FILE_ATTRIBUTE_SYSTEM != 0,
    )
}
