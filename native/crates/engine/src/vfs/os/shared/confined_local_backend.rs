//! Backend operations preserve the retained local directory authority.
use super::*;

impl Backend for ConfinedLocalBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        self.display.clone()
    }
    fn state_identity(&self) -> String {
        self.identity.clone()
    }
    fn namespace_identity(&self) -> String {
        "local".into()
    }
    fn flow_key(&self, _path: &str) -> String {
        self.flow.clone()
    }
    fn flow_identity(&self, path: &str) -> String {
        self.flow_key(path)
    }
    fn is_local(&self) -> bool {
        true
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
    fn root_confinement(&self, root: &str) -> RootConfinement {
        if root.trim_end_matches('/') == self.prefix {
            RootConfinement::Enforced
        } else {
            RootConfinement::Unverified
        }
    }
    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        StagedWriteCapabilities::complete()
    }
    fn rename_overwrites(&self) -> bool {
        true
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let _optical = crate::source_media::acquire_file_current(
            self.root.file(),
            crate::source_media::Priority::Foreground,
        )?;
        let listing = self.tolerant(path)?;
        if let Some(omitted) = listing
            .omitted
            .iter()
            .find(|item| item.reason != super::OmissionReason::Vanished)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                omitted.detail.clone(),
            ));
        }
        Ok(listing.entries)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let _optical = crate::source_media::acquire_file_current(
            self.root.file(),
            crate::source_media::Priority::Foreground,
        )?;
        let file = self.entry(path)?;
        let name = self
            .parts(path)?
            .last()
            .copied()
            .unwrap_or(path)
            .to_string();
        metadata(&file, name)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let file = self.file(path, false, false, false)?;
        crate::source_media::guard_file_reader(file.file.try_clone()?, file)
    }
    fn open_read_at(
        &self,
        path: &str,
        _id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        let mut file = self.file(path, false, false, false)?;
        file.file.seek(SeekFrom::Start(offset))?;
        crate::source_media::guard_file_reader(file.file.try_clone()?, file).map(Some)
    }
    fn supports_range_reads(&self, _path: &str) -> bool {
        matches!(crate::source_media::probe_file(self.root.file()), Ok(None))
    }
    fn open_read_range(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
        len: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        Ok(self
            .open_read_at(path, id, offset)?
            .map(|reader| Box::new(reader.take(len)) as Box<dyn Read + Send>))
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(self.file(path, true, false, true)?))
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(self.file(path, true, true, false)?))
    }
    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(self.sized_stage(path, size, true)?))
    }
    fn open_write_copy_stage_unsynced(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(self.sized_stage(path, size, false)?))
    }
    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.rename_inner(source, destination, true)
    }
    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.rename_inner(source, destination, false)
    }
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        super::promote_staged_with(self, staged, destination, |from, to| {
            self.rename_inner(from, to, true)
        })
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        let (parent, name) = self.parent(path)?;
        parent.remove(OsStr::new(&name), false)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        let (parent, name) = self.parent(path)?;
        parent.remove(OsStr::new(&name), true)
    }
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        super::super::local_platform::check_new_name(&super::super::local_platform::to_os(path))?;
        let (parent, name) = self.parent(path)?;
        parent.create_dir(OsStr::new(&name)).map(|_| ())
    }
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        if self.parts(path)?.is_empty() {
            return Ok(());
        }
        match self.create_dir_new(path) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                self.directory(path).map(|_| ())
            }
            result => result,
        }
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        super::super::local_platform::check_new_name(&super::super::local_platform::to_os(path))?;
        let mut directory = self.root.clone();
        for name in self.parts(path)? {
            directory = match directory.child(OsStr::new(name)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match directory.create_dir(OsStr::new(name)) {
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            directory.child(OsStr::new(name))?
                        }
                        result => result?,
                    }
                }
                result => result?,
            };
        }
        Ok(())
    }
    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        let file = self.file(stage, false, false, false)?;
        self.remove_file(stage)?;
        drop(file);
        Ok(())
    }
    fn copy_file(&self, source: &str, destination: &str) -> VfsResult<u64> {
        let stage = super::unique_staging_path(self, destination, "copy")?;
        let result = (|| {
            let mut reader = self.file(source, false, false, false)?;
            let mut writer = self.file(&stage, true, true, false)?;
            let copied = copy_data(
                &mut reader.file,
                &mut writer.file,
                &AtomicBool::new(false),
                None,
            )?;
            let destination_file = match self.entry(destination) {
                Ok(file) => Some(file),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            platform::copy_permissions(&reader.file, &writer.file, destination_file.as_ref())?;
            writer.flush()?;
            drop(writer);
            self.promote_staged(&stage, destination)?;
            Ok(copied)
        })();
        if result.is_err() {
            let _ = self.remove_file(&stage);
        }
        result
    }
    fn server_copy_to_stage(
        &self,
        source: &str,
        stage: &str,
        size: u64,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<u64>> {
        let mut reader = self.file(source, false, false, false)?;
        let mut writer = self.file(stage, true, true, false)?;
        let result =
            copy_data(&mut reader.file, &mut writer.file, cancel, Some(size)).and_then(|copied| {
                if copied != size || reader.file.metadata()?.len() != size {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "copy source length changed",
                    ));
                }
                writer.flush()?;
                Ok(copied)
            });
        if result.is_err() {
            drop(writer);
            let _ = self.remove_file(stage);
        }
        result.map(Some)
    }
}

// The source authority remains the retained descriptor. The device guard
// ends before namespace publication can call Backend::stat again.
fn copy_data(
    reader: &mut File,
    writer: &mut File,
    cancel: &AtomicBool,
    expected: Option<u64>,
) -> io::Result<u64> {
    let guard = crate::source_media::acquire_file(
        reader,
        crate::source_media::Priority::Foreground,
        cancel,
    )?;
    let Some(guard) = guard else {
        return platform::copy(reader, writer, cancel, expected);
    };
    guard.set_mode(crate::source_media::ReadMode::Sequential);
    let bytes = crate::source_media::sequential_copy(reader, writer, cancel, &mut |_| {})?
        .ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "optical copy canceled"))?;
    guard.validate_file(reader)?;
    if expected.is_some_and(|expected| bytes != expected) {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "optical source length changed",
        ));
    }
    Ok(bytes)
}
