//! Own-ID creation and cleanup for the live Google test. No endpoint override.
use super::api::{drive_request, mutation_once, FOLDER_MIME};
use super::identity::{in_parent, text};
use super::new_object::NewObject;
use super::sync_reliability_task_live_tests::checked;
use super::GDriveBackend;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::io;

struct OwnedObject {
    id: String,
    parent: String,
    title: String,
}

pub(super) struct LiveTree {
    pub(super) backend: GDriveBackend,
    pub(super) root_id: String,
    pub(super) root_path: String,
    own: Vec<OwnedObject>,
    cleaned: bool,
}

impl LiveTree {
    pub(super) fn create(backend: GDriveBackend) -> Self {
        let parent = checked(backend.actual_parent_id("root"), "resolve real My Drive ID");
        let id = checked(backend.take_generated_id(), "reserve own test root ID");
        let title = format!(
            "SmartExplorer-SyncReliability-{}",
            checked(super::overload::random_token(), "make root nonce")
        );
        let root_path = format!("/{}", super::names::encode(&title));
        let tree = Self {
            backend,
            root_id: id.clone(),
            root_path,
            own: vec![OwnedObject {
                id: id.clone(),
                parent: parent.clone(),
                title: title.clone(),
            }],
            cleaned: false,
        };
        // Root ID is captured before the POST, so an uncertain ACK still has
        // an exact cleanup target. These are test-owned IDs, never secrets.
        eprintln!("C10 captured own root ID: {id}");
        checked(
            tree.submit_folder(&id, &parent, &title),
            "create own test root",
        );
        tree
    }

    fn submit_folder(&self, id: &str, parent: &str, title: &str) -> io::Result<()> {
        let bearer = format!("Bearer {}", self.backend.bearer()?);
        let payload =
            json!({"id":id,"name":title,"parents":[parent],"mimeType":FOLDER_MIME}).to_string();
        let result = mutation_once(drive_request(
            self.backend
                .timed_request(self.backend.http.api().post(&self.backend.api_url(&format!(
                    "files?fields={}&supportsAllDrives=true",
                    super::identity::OBJECT_FIELDS
                ))))
                .set("Authorization", &bearer)
                .set("Content-Type", "application/json")
                .send_string(&payload),
        ));
        let failure = match result {
            Ok(response) => {
                super::api::drain(response);
                None
            }
            Err(error) => Some(error.into_io()),
        };
        if self.backend.folder_evidence(parent, title, id)?.is_some() {
            return Ok(());
        }
        Err(failure.unwrap_or_else(|| {
            io::Error::other("own folder create lacked exact identity evidence")
        }))
    }

    fn verify_parent(&self, id: &str) {
        assert!(
            self.own.iter().any(|object| object.id == id),
            "parent was created by this fixture"
        );
        checked(self.within_root(id), "verify owned mutation parent");
        assert_eq!(
            checked(self.backend.object_json(id), "verify parent type")["mimeType"],
            FOLDER_MIME
        );
    }

    pub(super) fn folder(&mut self, parent: &str, title: &str) -> String {
        self.verify_parent(parent);
        let id = checked(self.backend.take_generated_id(), "reserve own folder ID");
        self.own.push(OwnedObject {
            id: id.clone(),
            parent: parent.into(),
            title: title.into(),
        });
        checked(self.submit_folder(&id, parent, title), "create own folder");
        id
    }

    pub(super) fn file(&mut self, parent: &str, title: &str, bytes: &[u8]) -> String {
        self.verify_parent(parent);
        let id = checked(self.backend.take_generated_id(), "reserve own file ID");
        self.own.push(OwnedObject {
            id: id.clone(),
            parent: parent.into(),
            title: title.into(),
        });
        let object = NewObject {
            id: id.clone(),
            parent_id: parent.into(),
            title: title.into(),
            declare_binary: true,
            mtime_ms: None,
        };
        let hash = format!("{:x}", md5::compute(bytes));
        checked(
            self.backend.create_multipart(
                &object,
                io::Cursor::new(bytes),
                bytes.len() as u64,
                &hash,
            ),
            "create own exact-ID file",
        );
        id
    }

    fn within_root(&self, id: &str) -> io::Result<()> {
        let mut current = id.to_string();
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(current.clone()) {
                return Err(io::Error::other("owned parent chain is cyclic"));
            }
            let owned = self
                .own
                .iter()
                .find(|object| object.id == current)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "ID was not captured by this fixture",
                    )
                })?;
            let metadata = self.backend.object_json(&current)?;
            if !in_parent(&metadata, &owned.parent)? || text(&metadata, "name")? != owned.title {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "owned ID moved outside its recorded place",
                ));
            }
            if current == self.root_id {
                return Ok(());
            }
            current = owned.parent.clone();
        }
    }

    pub(super) fn snapshot(&self) -> BTreeMap<String, Value> {
        self.own
            .iter()
            .map(|object| {
                (
                    object.id.clone(),
                    checked(
                        self.backend.object_json(&object.id),
                        "capture owned metadata oracle",
                    ),
                )
            })
            .collect()
    }

    pub(super) fn cleanup(&mut self) -> io::Result<()> {
        if self.cleaned {
            return Ok(());
        }
        // Every mutation targets a captured own ID and a freshly verified
        // original parent chain. Never enumerate/trash the user's account.
        for object in self.own.iter().rev() {
            let json = match self.backend.object_json(&object.id) {
                Ok(json) => json,
                Err(error) if super::overload::http_status(&error) == Some(404) => continue,
                Err(error) => return Err(error),
            };
            if json["trashed"] == true {
                continue;
            }
            self.within_root(&object.id)?;
            self.backend.trash_id(&object.id)?;
            if self.backend.object_json(&object.id)?["trashed"] != true {
                return Err(io::Error::other("own ID is not confirmed trashed"));
            }
        }
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for LiveTree {
    fn drop(&mut self) {
        if self.cleanup().is_err() {
            eprintln!("C10 CLEANUP BLOCKER for captured own root {}", self.root_id);
            if !std::thread::panicking() {
                panic!("C10 own-ID cleanup failed");
            }
        }
    }
}
