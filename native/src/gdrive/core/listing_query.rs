//! One complete candidate collection for browsing, resolution and mutation.
use super::core::cloud_urlenc;
use super::file_list::FileListPage;
use super::identity::{in_parent, invalid, text, OBJECT_FIELDS};
use super::overload::http_status;
use super::GDriveBackend;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::io;

enum AttemptError {
    Terminal(io::Error),
    Restart(io::Error, bool),
}

impl From<io::Error> for AttemptError {
    fn from(error: io::Error) -> Self { Self::Terminal(error) }
}

impl GDriveBackend {
    pub(super) fn collect_files(&self, parent: &str, name: Option<&str>) -> io::Result<Vec<Value>> {
        let parent = self.actual_parent_id(parent)?;
        let mut query = format!("'{}' in parents", query_literal(&parent));
        if let Some(name) = name {
            query.push_str(&format!(" and name = '{}'", query_literal(name)));
        }
        query.push_str(" and trashed = false");
        let mut corpus = String::new();
        let mut backoff = super::api::listing_backoff();
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.collect_attempt(&parent, name, &query, &corpus) {
                Ok(files) => return Ok(files),
                Err(AttemptError::Terminal(error)) => return Err(error),
                Err(AttemptError::Restart(error, incomplete)) => {
                    if !super::api::restart_listing_read(attempt, &mut backoff, &error) {
                        return Err(error);
                    }
                    if incomplete {
                        corpus = self.narrowed_listing_corpus(&parent)?;
                    }
                }
            }
        }
    }

    fn collect_attempt(
        &self,
        parent: &str,
        name: Option<&str>,
        query: &str,
        corpus: &str,
    ) -> Result<Vec<Value>, AttemptError> {
        let mut output = BTreeMap::<String, Value>::new();
        let mut observed = BTreeMap::<String, Value>::new();
        let mut conflicting = HashSet::new();
        let mut tokens = HashSet::new();
        let mut token: Option<String> = None;
        loop {
            let mut url = self.api_url(&format!(
                "files?q={}&fields=nextPageToken,incompleteSearch,files({OBJECT_FIELDS})&pageSize=1000&supportsAllDrives=true&includeItemsFromAllDrives=true{corpus}",
                cloud_urlenc(query)
            ));
            if let Some(token) = &token {
                url.push_str(&format!("&pageToken={}", cloud_urlenc(token)));
            }
            let json = match self.get_json(&url) {
                Ok(json) => json,
                Err(error) if token.is_some() && matches!(http_status(&error), Some(400 | 410)) => {
                    return Err(AttemptError::Restart(error, false));
                }
                Err(error) => return Err(error.into()),
            };
            let page = FileListPage::parse(&json)?;
            if page.incomplete {
                return Err(AttemptError::Restart(invalid("Drive search is incomplete"), true));
            }
            for file in page.files {
                // Search equality is only a candidate filter, including case
                // variants. Other literal titles are not duplicate identities.
                let title = text(file, "name")?;
                let id = text(file, "id")?.to_string();
                if let Some(previous) = observed.insert(id.clone(), file.clone()) {
                    if previous != *file {
                        conflicting.insert(id.clone());
                    }
                }
                if name.is_some_and(|expected| title != expected) {
                    continue;
                }
                if !in_parent(file, parent)? {
                    continue;
                }
                output.entry(id).or_insert_with(|| file.clone());
            }
            token = page.next_token.map(str::to_owned);
            let Some(next) = &token else { break; };
            if !tokens.insert(next.clone()) {
                return Err(AttemptError::Restart(invalid("Drive repeated a listing page token"), false));
            }
        }
        // Overlapping pages identify one object. Contradictory observations
        // need a fresh exact-ID read; they never prove another sibling exists.
        for id in conflicting {
            if !output.contains_key(&id) { continue; }
            match self.object_json(&id) {
                Ok(file) => {
                    let matches = in_parent(&file, parent)?
                        && name.is_none_or(|expected| file["name"].as_str() == Some(expected));
                    if matches { output.insert(id, file); } else { output.remove(&id); }
                }
                Err(error) if http_status(&error) == Some(404) => { output.remove(&id); }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(output.into_values().collect())
    }
}

pub(super) fn query_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}
