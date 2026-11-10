//! Drive v3 REST: list, download or export, create, and update with a conflict check.

use std::time::Instant;

use crate::encode::{drive_literal, form, pct};
use crate::http::{Method, Request, Response};
use crate::oauth::Auth;
use crate::{Error, Result};

pub const API: &str = "https://www.googleapis.com/drive/v3";
pub const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3";

pub const MIME_DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
pub const MIME_ODT: &str = "application/vnd.oasis.opendocument.text";
pub const MIME_RTF: &str = "application/rtf";
pub const MIME_GOOGLE_DOC: &str = "application/vnd.google-apps.document";

/// The most bytes downloaded or uploaded for one document.
pub const MAX_DOCUMENT: u64 = 256 << 20;
/// The most files one listing returns.
pub const PAGE_SIZE: u32 = 100;

const FIELDS: &str = "id,name,mimeType,modifiedTime,version,size,headRevisionId";

/// A file's metadata, as much as WordCraft needs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    /// RFC 3339, shown as is.
    #[serde(default)]
    pub modified_time: String,
    /// Drive's change counter (an int64 sent as a string); the conflict check compares it.
    #[serde(default)]
    pub version: String,
    /// Bytes; absent for Google Docs.
    #[serde(default, deserialize_with = "opt_u64_str")]
    pub size: Option<u64>,
}

fn opt_u64_str<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<u64>, D::Error> {
    let s: Option<String> = serde::Deserialize::deserialize(d)?;
    Ok(s.and_then(|s| s.parse().ok()))
}

impl DriveFile {
    /// A native Google Doc: opened by exporting it as `.docx`, never overwritten.
    pub fn is_google_doc(&self) -> bool {
        self.mime_type == MIME_GOOGLE_DOC
    }
    /// The name WordCraft opens it under, with an extension its format dispatch understands.
    pub fn open_name(&self) -> String {
        if self.is_google_doc() && !self.name.to_ascii_lowercase().ends_with(".docx") { format!("{}.docx", self.name) } else { self.name.clone() }
    }
}

/// Where a save goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveTarget {
    /// Replace the contents of a file opened from Drive, if it is still at `version`.
    Existing { id: String, name: String, version: String },
    /// A new file, in `parent` (a folder id) or My Drive.
    New { name: String, parent: Option<String> },
}

/// The MIME type for a file name WordCraft saves.
pub fn mime_for(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".odt") {
        MIME_ODT
    } else if lower.ends_with(".rtf") {
        MIME_RTF
    } else if lower.ends_with(".txt") {
        "text/plain"
    } else {
        MIME_DOCX
    }
}

/// Drive ids are URL-safe base64-ish; anything else is refused before it reaches a URL.
pub fn check_id(id: &str) -> Result<&str> {
    if !id.is_empty() && id.len() <= 256 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        Ok(id)
    } else {
        Err(Error::Invalid(format!("not a Google Drive file id: {:?}", id.chars().take(40).collect::<String>())))
    }
}

fn check_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
        return Err(Error::Invalid("a Google Drive file name can't be empty, very long or contain control characters".into()));
    }
    Ok(name)
}

/// The listing's search expression: documents WordCraft opens, not in the trash, optionally
/// whose name contains `search`.
pub fn list_query(search: Option<&str>) -> String {
    let types =
        [MIME_DOCX, MIME_GOOGLE_DOC, MIME_ODT, MIME_RTF].iter().map(|m| format!("mimeType = {}", drive_literal(m))).collect::<Vec<_>>().join(" or ");
    let mut q = format!("trashed = false and ({types})");
    if let Some(s) = search.map(str::trim).filter(|s| !s.is_empty()) {
        let s: String = s.chars().take(200).collect();
        q.push_str(&format!(" and name contains {}", drive_literal(&s)));
    }
    q
}

/// The `multipart/related` body for a create: JSON metadata, then the content.
pub fn multipart(boundary: &str, metadata: &serde_json::Value, mime: &str, content: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(content.len() + 512);
    body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes());
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Type: {mime}\r\n\r\n").as_bytes());
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// A signed-in Drive client.
pub struct Drive {
    pub auth: Auth,
}

impl Drive {
    pub fn new(auth: Auth) -> Self {
        Self { auth }
    }

    fn send(&mut self, req: Request) -> Result<Response> {
        let token = self.auth.access_token(Instant::now())?;
        let resp = self.auth.http().send(req.bearer(&token))?;
        if resp.status == 401 {
            // Revoked between the expiry check and the call.
            return Err(Error::NotSignedIn);
        }
        Ok(resp)
    }

    fn file(resp: Response) -> Result<DriveFile> {
        let v = resp.json()?;
        serde_json::from_value(v).map_err(|e| Error::Protocol(format!("file metadata: {e}")))
    }

    /// One page of documents, newest first, and the token for the next page.
    pub fn list(&mut self, search: Option<&str>, page_token: Option<&str>) -> Result<(Vec<DriveFile>, Option<String>)> {
        let size = PAGE_SIZE.to_string();
        let fields = format!("nextPageToken,files({FIELDS})");
        let q = list_query(search);
        let mut pairs =
            vec![("q", q.as_str()), ("fields", fields.as_str()), ("orderBy", "modifiedTime desc"), ("pageSize", size.as_str()), ("spaces", "drive")];
        if let Some(t) = page_token.filter(|t| !t.is_empty() && t.len() <= 4096) {
            pairs.push(("pageToken", t));
        }
        let v = self.send(Request::new(Method::Get, format!("{API}/files?{}", form(&pairs))))?.json()?;
        let files = v
            .get("files")
            .and_then(|f| f.as_array())
            .map(|a| {
                a.iter()
                    .take(PAGE_SIZE as usize)
                    .filter_map(|f| serde_json::from_value::<DriveFile>(f.clone()).ok())
                    .filter(|f| check_id(&f.id).is_ok())
                    .collect()
            })
            .unwrap_or_default();
        let next = v.get("nextPageToken").and_then(|t| t.as_str()).map(str::to_owned);
        Ok((files, next))
    }

    pub fn metadata(&mut self, id: &str) -> Result<DriveFile> {
        let id = check_id(id)?;
        let url = format!("{API}/files/{id}?fields={}", pct(FIELDS));
        Self::file(self.send(Request::new(Method::Get, url))?)
    }

    /// The file's bytes: the content for uploaded files, a `.docx` export for Google Docs.
    pub fn download(&mut self, file: &DriveFile) -> Result<Vec<u8>> {
        let id = check_id(&file.id)?;
        if file.size.is_some_and(|s| s > MAX_DOCUMENT) {
            return Err(Error::Invalid(format!("{} is larger than {} MB", file.name, MAX_DOCUMENT >> 20)));
        }
        let url =
            if file.is_google_doc() { format!("{API}/files/{id}/export?mimeType={}", pct(MIME_DOCX)) } else { format!("{API}/files/{id}?alt=media") };
        let resp = self.send(Request::new(Method::Get, url).limit(MAX_DOCUMENT))?;
        if !resp.ok() {
            return Err(resp.api_error());
        }
        Ok(resp.body)
    }

    /// Saves `content` and returns the file's new metadata.
    ///
    /// For an existing file, Drive's current `version` is read first; if it moved since the file
    /// was opened the save is refused with [`Error::Conflict`] and nothing is written. Drive v3
    /// has no conditional update, so a change landing between that read and the upload (well
    /// under a second) is not caught. Google Docs are never overwritten: save them as a new file.
    pub fn save(&mut self, target: &SaveTarget, content: &[u8]) -> Result<DriveFile> {
        if content.len() as u64 > MAX_DOCUMENT {
            return Err(Error::Invalid(format!("documents larger than {} MB can't be saved to Drive", MAX_DOCUMENT >> 20)));
        }
        match target {
            SaveTarget::Existing { id, name, version } => {
                let id = check_id(id)?;
                let now = self.metadata(id)?;
                if now.is_google_doc() {
                    return Err(Error::Invalid(format!("{} is a Google Doc; save a .docx copy instead", now.name)));
                }
                if now.version != *version {
                    return Err(Error::Conflict { name: name.clone(), opened: version.clone(), remote: now.version });
                }
                let url = format!("{UPLOAD_API}/files/{id}?uploadType=media&fields={}", pct(FIELDS));
                let req = Request::new(Method::Patch, url).body(mime_for(&now.name), content.to_vec());
                Self::file(self.send(req)?)
            }
            SaveTarget::New { name, parent } => {
                let name = check_name(name)?;
                let mime = mime_for(name);
                let mut meta = serde_json::json!({ "name": name, "mimeType": mime });
                if let Some(p) = parent {
                    meta["parents"] = serde_json::json!([check_id(p)?]);
                }
                let boundary = format!("wordcraft-{}", crate::pkce::random_token(18)?);
                let body = multipart(&boundary, &meta, mime, content);
                let url = format!("{UPLOAD_API}/files?uploadType=multipart&fields={}", pct(FIELDS));
                let req = Request::new(Method::Post, url).body(&format!("multipart/related; boundary={boundary}"), body);
                Self::file(self.send(req)?)
            }
        }
    }
}
