//! Private, seekable result storage: encoding once does not retain a second tree.
use crate::share::{
    analysis_resources::Reservation,
    framing,
    wire::{Ctrl, FsResponse},
};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    sync::Mutex,
};

pub(in crate::share) struct Writer {
    file: File,
    bytes: u64,
    hold: Reservation,
}
pub(in crate::share) struct Spool {
    file: Mutex<File>,
    pub(in crate::share) bytes: u64,
    _hold: Reservation,
}

impl Writer {
    pub(in crate::share) fn new() -> io::Result<Self> {
        let dir = crate::support_dirs::app_data_dir().join("analysis-cache");
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            file: tempfile::tempfile_in(dir)?,
            bytes: 0,
            hold: Reservation::disk(),
        })
    }

    pub(in crate::share) fn control(&mut self, response: FsResponse) -> io::Result<()> {
        let bytes =
            serde_json::to_vec(&Ctrl::FsResp { resp: response }).map_err(io::Error::other)?;
        self.frame(framing::TAG_CTRL, &bytes)
    }

    pub(in crate::share) fn frame(&mut self, tag: u8, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Analyse-Portion überschreitet das Drahtformat",
            ));
        }
        let size = u32::try_from(bytes.len()).map_err(io::Error::other)?;
        self.hold.add(5 + u64::from(size))?;
        self.file.write_all(&size.to_le_bytes())?;
        self.file.write_all(&[tag])?;
        self.file.write_all(bytes)?;
        self.bytes = self
            .bytes
            .checked_add(5 + u64::from(size))
            .ok_or_else(|| io::Error::other("Analyse-Zwischendatei zu groß"))?;
        Ok(())
    }

    pub(in crate::share) fn finish(mut self) -> io::Result<Spool> {
        self.file.flush()?;
        Ok(Spool {
            file: Mutex::new(self.file),
            bytes: self.bytes,
            _hold: self.hold,
        })
    }
}

impl Spool {
    pub(in crate::share) fn frame(&self, offset: &mut u64) -> io::Result<Option<(u8, Vec<u8>)>> {
        if *offset == self.bytes {
            return Ok(None);
        }
        let mut file = self
            .file
            .lock()
            .map_err(|_| io::Error::other("Analyse-Zwischendatei gesperrt"))?;
        file.seek(SeekFrom::Start(*offset))?;
        let mut header = [0; 5];
        file.read_exact(&mut header)?;
        let size = u32::from_le_bytes(header[..4].try_into().map_err(io::Error::other)?) as usize;
        if size > 4 * 1024 * 1024 || *offset + 5 + size as u64 > self.bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Analyse-Zwischendatei beschädigt",
            ));
        }
        let mut bytes = vec![0; size];
        file.read_exact(&mut bytes)?;
        *offset += 5 + size as u64;
        Ok(Some((header[4], bytes)))
    }
}
