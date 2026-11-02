use super::*;
use windows::core::Interface;

#[test]
fn search_recursive_access_task_clipboard_stream_read_seek_clone_and_denied_write() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("asset.blend");
    std::fs::write(&path, b"0123456789").unwrap();
    let stream = open(&path.to_string_lossy()).unwrap();
    let sequential = stream.cast::<ISequentialStream>().unwrap();
    unsafe {
        let mut bytes = [0u8; 4];
        let mut read = 0;
        assert_eq!(
            sequential.Read(bytes.as_mut_ptr().cast(), 4, Some(&mut read)),
            S_OK
        );
        assert_eq!((&bytes, read), (b"0123", 4));
        let cloned = stream.Clone().unwrap();
        stream.Seek(1, STREAM_SEEK_SET, None).unwrap();
        assert_eq!(
            cloned.Read(bytes.as_mut_ptr().cast(), 4, Some(&mut read)),
            S_OK
        );
        assert_eq!(&bytes, b"4567");
        assert_eq!(
            stream.Read(bytes.as_mut_ptr().cast(), 4, Some(&mut read)),
            S_OK
        );
        assert_eq!(&bytes, b"1234");
        stream.Seek(-2, STREAM_SEEK_END, None).unwrap();
        assert_eq!(
            stream.Read(bytes.as_mut_ptr().cast(), 4, Some(&mut read)),
            S_FALSE
        );
        assert_eq!(read, 2);
        assert_eq!(&bytes[..2], b"89");
        let mut written = 123;
        assert_eq!(
            stream.Write(b"bad".as_ptr().cast(), 3, Some(&mut written)),
            STG_E_ACCESSDENIED
        );
        assert_eq!(written, 0);
        assert!(stream.SetSize(0).is_err());
        assert!(stream.Seek(-1, STREAM_SEEK_SET, None).is_err());
        let mut metadata = STATSTG::default();
        stream.Stat(&mut metadata, STATFLAG_NONAME).unwrap();
        assert_eq!(metadata.cbSize, 10);
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"0123456789");
}
