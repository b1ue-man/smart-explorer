use super::GuardedBackend;
use crate::vfs::{self, BackendExtensions, VfsResult};
use std::{
    io::{Read, Write},
    sync::atomic::AtomicBool,
    time::Duration,
};

impl BackendExtensions for GuardedBackend {
    fn previous_state_identities(&self) -> VfsResult<Vec<String>> {
        self.read(&self.inner.root_display())?;
        let identities = vfs::previous_state_identities(&*self.inner)?;
        self.read(&self.inner.root_display())?;
        Ok(identities)
    }
    fn sync_child_path(&self, parent: &str, literal_name: &str) -> VfsResult<String> {
        self.read(parent)?;
        if super::super::fs_policy::private_name(literal_name) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Pfad ist nicht freigegeben",
            ));
        }
        let child = vfs::sync_child_path(&*self.inner, parent, literal_name)?;
        self.read(&child)?;
        Ok(child)
    }
    fn replace_staged_reversible(
        &self,
        staged: &str,
        destination: &str,
        retained: &str,
    ) -> VfsResult<bool> {
        self.destructive(staged)?;
        self.destructive(destination)?;
        self.destructive(retained)?;
        let replaced = vfs::replace_staged_reversible(&*self.inner, staged, destination, retained)?;
        self.write(destination)?;
        Ok(replaced)
    }
    fn list_dir_tolerant(&self, path: &str) -> VfsResult<vfs::VfsListing> {
        self.read(path)?;
        let mut listing = vfs::list_dir_tolerant(&*self.inner, path)?;
        listing.entries = self.filter(path, listing.entries);
        listing
            .omitted
            .retain(|entry| !super::super::fs_policy::private_path(&entry.rel));
        Ok(listing)
    }
    fn open_read_regular(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.read(path)?;
        Ok(self.reader(vfs::open_read_regular(&*self.inner, path, id)?))
    }
    fn open_write_copy_stage_timed(
        &self,
        path: &str,
        size: u64,
        mtime_ms: i64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.write(path)?;
        Ok(self.writer(vfs::open_write_copy_stage_timed(
            &*self.inner,
            path,
            size,
            mtime_ms,
        )?))
    }
    fn finish_stage(&self, stage: &str, finish: vfs::StageFinish) -> VfsResult<vfs::StageFinished> {
        self.write(stage)?;
        vfs::finish_stage(&*self.inner, stage, finish)
    }
    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        self.write(root)?;
        vfs::sync_filesystem(&*self.inner, root)
    }
    fn confirm_namespace(&self, parent: &str) -> VfsResult<bool> {
        self.write(parent)?;
        let durable = vfs::confirm_namespace(&*self.inner, parent)?;
        self.write(parent)?;
        Ok(durable)
    }
    fn target_limits(&self, root: &str) -> vfs::TargetLimits {
        vfs::target_limits(&*self.inner, root)
    }
    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        self.read(path)?;
        vfs::unix_mode(&*self.inner, path)
    }
    fn volume_identity(&self, root: &str) -> VfsResult<Option<vfs::VolumeIdentity>> {
        self.read(root)?;
        vfs::volume_identity(&*self.inner, root)
    }
    fn supports_duplicate_search(&self, root: &str) -> VfsResult<bool> {
        self.read(root)?;
        vfs::supports_duplicate_search(&*self.inner, root)
    }
    fn find_duplicates(
        &self,
        root: &str,
        min_bytes: u64,
        progress: &crate::analytics::ReclaimProgress,
    ) -> VfsResult<Option<crate::analytics::DuplicateReport>> {
        self.read(root)?;
        if let Some(authority) = &self.authority {
            authority.register_cancel(&progress.cancel)?;
        }
        let report = vfs::find_duplicates(&*self.inner, root, min_bytes, progress)?;
        self.read(root)?;
        Ok(report.map(|report| super::reports::duplicates(report, &self.policy)))
    }
    fn supports_hash_walk(&self, root: &str) -> VfsResult<bool> {
        self.read(root)?;
        vfs::supports_hash_walk(&*self.inner, root)
    }
    fn hash_walk(
        &self,
        root: &str,
        request: vfs::HashWalkRequest,
        tx: crossbeam_channel::Sender<vfs::HashWalkItem>,
        cancel: &AtomicBool,
    ) -> VfsResult<bool> {
        self.read(root)?;
        super::stream::forward(
            self.authority.as_ref(),
            &self.policy,
            root,
            tx,
            cancel,
            |item| match item {
                vfs::HashWalkItem::Entry(entry) => &entry.rel,
                vfs::HashWalkItem::Omitted(omission) => &omission.rel,
            },
            |output, cancelled| vfs::hash_walk(&*self.inner, root, request, output, cancelled),
        )
    }
    fn supports_recycle(&self, path: &str) -> VfsResult<bool> {
        self.read(path)?;
        vfs::supports_recycle(&*self.inner, path)
    }
    fn recycle(
        &self,
        path: &str,
        expected: &vfs::RecycleExpectation,
    ) -> VfsResult<vfs::RecycleOutcome> {
        self.destructive(path)?;
        vfs::recycle(&*self.inner, path, expected)
    }
    fn change_signal_mode(&self, root: &str) -> VfsResult<Option<vfs::ChangeSignalMode>> {
        self.read(root)?;
        vfs::change_signal_mode(&*self.inner, root)
    }
    fn change_signal(
        &self,
        root: &str,
        poll_interval: Duration,
        tx: crossbeam_channel::Sender<vfs::ChangeNotice>,
    ) -> VfsResult<Option<vfs::ChangeSubscription>> {
        self.read(root)?;
        super::stream::subscribe(
            self.authority.clone(),
            self.policy.clone(),
            root.to_owned(),
            tx,
            |output| vfs::change_signal(&*self.inner, root, poll_interval, output),
        )
    }
}
