//! A small in-memory Drive v3 for the transfer-engine task tests: generated
//! IDs, folder creates, multipart and resumable uploads, name queries, exact-ID
//! lookups, trash and copy. Answers mirror the fields the backend requests.
use super::api::FOLDER_MIME;
use super::task_http::{Answer, Request, Server};
use super::GDriveBackend;
use crate::vfs::Backend;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Open upload sessions: metadata, announced length and the bytes so far.
type Sessions = HashMap<String, (Value, u64, Vec<u8>)>;

pub(super) struct FakeDrive {
    pub(super) root_id: String,
    pub(super) permission_id: String,
    next: AtomicUsize,
    page_size: AtomicUsize,
    objects: Mutex<HashMap<String, Value>>,
    sessions: Mutex<Sessions>,
    media: Mutex<HashMap<String, Vec<u8>>>,
}

impl Default for FakeDrive {
    fn default() -> Self {
        static SERIAL: AtomicUsize = AtomicUsize::new(0);
        let serial = SERIAL.fetch_add(1, Ordering::SeqCst);
        Self::new(
            &format!("fake-root-{serial}"),
            &format!("fake-user-{serial}"),
        )
    }
}

impl FakeDrive {
    pub(super) fn new(root_id: &str, permission_id: &str) -> Self {
        Self {
            root_id: root_id.into(),
            permission_id: permission_id.into(),
            next: AtomicUsize::new(0),
            page_size: AtomicUsize::new(0),
            objects: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            media: Mutex::new(HashMap::new()),
        }
    }

    fn parent<'a>(&'a self, parent: &'a str) -> &'a str {
        if parent == "root" {
            &self.root_id
        } else {
            parent
        }
    }

    pub(super) fn set_page_size(&self, size: usize) {
        self.page_size.store(size, Ordering::SeqCst);
    }

    pub(super) fn change(&self, id: &str, changes: Value) {
        let mut objects = self.objects.lock().unwrap();
        let current = objects.get_mut(id).expect("fixture object exists");
        for (key, value) in changes.as_object().unwrap() {
            current[key] = value.clone();
        }
    }

    pub(super) fn bytes(&self, id: &str) -> Vec<u8> {
        self.media
            .lock()
            .unwrap()
            .get(id)
            .expect("fixture media exists")
            .clone()
    }

    pub(super) fn insert(&self, id: &str, name: &str, parent: &str, mime: &str, content: &[u8]) {
        self.media
            .lock()
            .unwrap()
            .insert(id.to_string(), content.to_vec());
        self.objects.lock().unwrap().insert(
            id.to_string(),
            object(id, name, self.parent(parent), mime, Some(content)),
        );
    }

    pub(super) fn object(&self, id: &str) -> Option<Value> {
        if id == "root" || id == self.root_id {
            return Some(
                json!({"id": self.root_id, "name": "My Drive", "parents": [],
                "mimeType": FOLDER_MIME, "trashed": false,
                "modifiedTime": "2026-10-02T12:34:56.789Z"}),
            );
        }
        self.objects.lock().unwrap().get(id).cloned()
    }

    pub(super) fn named(&self, parent: &str, name: &str) -> Vec<Value> {
        let parent = self.parent(parent);
        let mut named: Vec<Value> = self
            .objects
            .lock()
            .unwrap()
            .values()
            .filter(|object| {
                object["name"] == name
                    && object["parents"] == json!([parent])
                    && object["trashed"] == false
            })
            .cloned()
            .collect();
        named.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
        named
    }

    pub(super) fn children(&self, parent: &str) -> Vec<Value> {
        let parent = self.parent(parent);
        let mut children: Vec<Value> = self
            .objects
            .lock()
            .unwrap()
            .values()
            .filter(|object| object["parents"] == json!([parent]) && object["trashed"] == false)
            .cloned()
            .collect();
        children.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
        children
    }

    pub(super) fn answer(&self, request: &Request) -> Answer {
        let path = request.path().to_string();
        match (request.method.as_str(), path.as_str()) {
            ("GET", "/drive/v3/about") => {
                Answer::json(json!({"user": {"permissionId": self.permission_id}}))
            }
            ("GET", "/drive/v3/files/generateIds") => {
                let count: usize = request
                    .query("count")
                    .and_then(|count| count.parse().ok())
                    .unwrap_or(10);
                let ids: Vec<String> = (0..count)
                    .map(|_| format!("gen-{}", self.next.fetch_add(1, Ordering::SeqCst)))
                    .collect();
                Answer::json(json!({ "ids": ids }))
            }
            ("POST", "/drive/v3/files") => {
                let metadata: Value = serde_json::from_slice(&request.body).unwrap();
                self.create(
                    &metadata,
                    metadata["mimeType"].as_str().unwrap_or(FOLDER_MIME),
                    None,
                )
            }
            ("POST", "/upload/drive/v3/files") => match request.query("uploadType").as_deref() {
                Some("multipart") => {
                    let (metadata, media) = multipart(request);
                    let mime = metadata["mimeType"]
                        .as_str()
                        .unwrap_or("application/octet-stream")
                        .to_string();
                    self.create(&metadata, &mime, Some(&media))
                }
                _ => {
                    let metadata: Value = serde_json::from_slice(&request.body).unwrap();
                    let total = request
                        .header("x-upload-content-length")
                        .unwrap()
                        .parse()
                        .unwrap();
                    let session = format!("s{}", self.next.fetch_add(1, Ordering::SeqCst));
                    self.sessions
                        .lock()
                        .unwrap()
                        .insert(session.clone(), (metadata, total, Vec::new()));
                    let location = format!(
                        "http://{}/upload/session/{session}",
                        request.header("host").unwrap()
                    );
                    Answer::json(json!({})).header("Location", &location)
                }
            },
            ("PUT", session) if session.starts_with("/upload/session/") => self.put(request),
            ("PATCH", item) if item.starts_with("/upload/drive/v3/files/") => {
                let id = item.rsplit('/').next().unwrap();
                let Some(mut metadata) = self.object(id) else {
                    return Answer::status(404, json!({"error": {"message": "File not found"}}));
                };
                let requested: Value = serde_json::from_slice(&request.body).unwrap();
                if let Some(time) = requested["modifiedTime"].as_str() {
                    metadata["modifiedTime"] = json!(time);
                }
                metadata["_replace"] = json!(true);
                let total = request
                    .header("x-upload-content-length")
                    .unwrap()
                    .parse()
                    .unwrap();
                let session = format!("s{}", self.next.fetch_add(1, Ordering::SeqCst));
                self.sessions
                    .lock()
                    .unwrap()
                    .insert(session.clone(), (metadata, total, Vec::new()));
                Answer::json(json!({})).header(
                    "Location",
                    &format!(
                        "http://{}/upload/session/{session}",
                        request.header("host").unwrap()
                    ),
                )
            }
            ("GET", "/drive/v3/files") => {
                let query = request.query("q").unwrap_or_default();
                let parent = super::task_http::query_literal(query.trim_start_matches('\''));
                super::task_http::list_page(
                    request,
                    self.children(&parent),
                    self.page_size.load(Ordering::SeqCst),
                )
            }
            ("POST", copy) if copy.ends_with("/copy") => {
                let source = copy.trim_end_matches("/copy").rsplit('/').next().unwrap();
                let Some(source) = self.object(source) else {
                    return Answer::status(404, json!({"error": {"message": "File not found"}}));
                };
                let metadata: Value = serde_json::from_slice(&request.body).unwrap();
                let mut copy = source.clone();
                copy["id"] = metadata["id"].clone();
                copy["name"] = metadata["name"].clone();
                copy["parents"] = json!([self.parent(metadata["parents"][0].as_str().unwrap())]);
                self.objects
                    .lock()
                    .unwrap()
                    .insert(metadata["id"].as_str().unwrap().to_string(), copy.clone());
                let copied_bytes = self
                    .media
                    .lock()
                    .unwrap()
                    .get(source["id"].as_str().unwrap())
                    .cloned();
                if let Some(bytes) = copied_bytes {
                    self.media
                        .lock()
                        .unwrap()
                        .insert(metadata["id"].as_str().unwrap().to_string(), bytes);
                }
                Answer::json(copy)
            }
            ("GET", item) if item.starts_with("/drive/v3/files/") => {
                match self.object(item.rsplit('/').next().unwrap()) {
                    Some(object) if request.query("alt").as_deref() == Some("media") => Answer {
                        status: 200,
                        headers: Vec::new(),
                        body: self
                            .media
                            .lock()
                            .unwrap()
                            .get(object["id"].as_str().unwrap())
                            .cloned()
                            .unwrap_or_default(),
                    },
                    Some(object) => Answer::json(object),
                    None => Answer::status(404, json!({"error": {"message": "File not found"}})),
                }
            }
            ("PATCH", item) if item.starts_with("/drive/v3/files/") => {
                let id = item.rsplit('/').next().unwrap().to_string();
                let change: Value = serde_json::from_slice(&request.body).unwrap();
                let mut objects = self.objects.lock().unwrap();
                let Some(object) = objects.get_mut(&id) else {
                    return Answer::status(404, json!({"error": {"message": "File not found"}}));
                };
                if let Some(trashed) = change["trashed"].as_bool() {
                    object["trashed"] = json!(trashed);
                }
                if let Some(name) = change["name"].as_str() {
                    object["name"] = json!(name);
                }
                if let Some(time) = change["modifiedTime"].as_str() {
                    object["modifiedTime"] = json!(time);
                }
                if let Some(parent) = request.query("addParents") {
                    object["parents"] = json!([parent]);
                }
                Answer::json(object.clone())
            }
            _ => Answer::status(400, json!({"error": {"message": "unexpected request"}})),
        }
    }

    fn create(&self, metadata: &Value, mime: &str, content: Option<&[u8]>) -> Answer {
        let id = metadata["id"].as_str().unwrap().to_string();
        let mut objects = self.objects.lock().unwrap();
        if objects.contains_key(&id) {
            return Answer::status(409, json!({"error": {"message": "ID already exists"}}));
        }
        let mut created = object(
            &id,
            metadata["name"].as_str().unwrap(),
            self.parent(metadata["parents"][0].as_str().unwrap()),
            mime,
            content,
        );
        if let Some(time) = metadata["modifiedTime"].as_str() {
            created["modifiedTime"] = json!(time);
        }
        objects.insert(id, created.clone());
        if let Some(content) = content {
            self.media.lock().unwrap().insert(
                metadata["id"].as_str().unwrap().to_string(),
                content.to_vec(),
            );
        }
        Answer::json(created)
    }

    fn put(&self, request: &Request) -> Answer {
        let session = request.path().rsplit('/').next().unwrap().to_string();
        let mut sessions = self.sessions.lock().unwrap();
        let (metadata, total, data) = sessions.get_mut(&session).unwrap();
        let range = request
            .header("content-range")
            .unwrap_or_default()
            .to_string();
        if let Some(span) = range
            .strip_prefix("bytes ")
            .and_then(|span| span.split('/').next())
        {
            if let Some((start, _)) = span.split_once('-') {
                assert_eq!(
                    start.parse::<usize>().unwrap(),
                    data.len(),
                    "chunk starts at the confirmed offset"
                );
                data.extend_from_slice(&request.body);
            }
        }
        if data.len() as u64 == *total {
            let (metadata, content) = (metadata.clone(), data.clone());
            drop(sessions);
            if metadata["_replace"].as_bool() == Some(true) {
                let id = metadata["id"].as_str().unwrap();
                self.insert(
                    id,
                    metadata["name"].as_str().unwrap(),
                    metadata["parents"][0].as_str().unwrap(),
                    metadata["mimeType"].as_str().unwrap(),
                    &content,
                );
                if let Some(time) = metadata["modifiedTime"].as_str() {
                    self.objects.lock().unwrap().get_mut(id).unwrap()["modifiedTime"] = json!(time);
                }
                return Answer::json(self.object(id).unwrap());
            }
            return self.create(&metadata, "application/octet-stream", Some(&content));
        }
        match data.len() {
            0 => Answer::status(308, json!({})),
            kept => {
                Answer::status(308, json!({})).header("Range", &format!("bytes=0-{}", kept - 1))
            }
        }
    }
}

fn object(id: &str, name: &str, parent: &str, mime: &str, content: Option<&[u8]>) -> Value {
    let mut object = json!({
        "id": id, "name": name, "parents": [parent], "mimeType": mime, "trashed": false,
        "modifiedTime": "2026-10-02T12:34:56.789Z",
    });
    if let Some(content) = content {
        object["size"] = json!(content.len().to_string());
        object["md5Checksum"] = json!(format!("{:x}", md5::compute(content)));
    }
    object
}

pub(super) use super::task_http::multipart;

pub(super) fn drive_server() -> (Arc<FakeDrive>, Server) {
    let drive = Arc::new(FakeDrive::default());
    let server = {
        let drive = Arc::clone(&drive);
        Server::start(move |request| drive.answer(request))
    };
    (drive, server)
}

/// A backend whose folder "Job" was created by the running transfer.
pub(super) fn job_backend(server: &Server) -> GDriveBackend {
    let backend = server.backend();
    backend
        .remember_path("Job", "job-folder", Some(FOLDER_MIME))
        .unwrap();
    backend
}

pub(super) fn write_fresh(backend: &GDriveBackend, path: &str, content: &[u8]) -> io::Result<()> {
    let mut writer = backend
        .open_write_fresh(path, content.len() as u64)?
        .ok_or_else(|| io::Error::other("Drive must accept fresh files"))?;
    writer.write_all(content)?;
    writer.flush()
}

pub(super) fn count(requests: &[Request], method: &str, path: &str) -> usize {
    requests
        .iter()
        .filter(|request| request.method == method && request.path() == path)
        .count()
}

pub(super) fn drive_error(status: u16, reason: &str, message: &str) -> Answer {
    Answer::status(
        status,
        json!({"error": {"message": message, "errors": [{"reason": reason}]}}),
    )
}
