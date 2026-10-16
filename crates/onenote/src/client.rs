//! Thin blocking Microsoft Graph client for OneNote. Intended to run on a
//! background thread; every call is synchronous.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::Value;

use crate::auth::{self, AuthConfig, DeviceCode, Tokens};
use crate::multipart::{self, Part};
use crate::{Error, Result};

const GRAPH_V1: &str = "https://graph.microsoft.com/v1.0/me/onenote";
/// Ink read/write (InkML) is only exposed on the beta endpoint.
const GRAPH_BETA: &str = "https://graph.microsoft.com/beta/me/onenote";
const MAX_BODY: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSection {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteNotebook {
    pub id: String,
    pub name: String,
    pub sections: Vec<RemoteSection>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePage {
    pub id: String,
    pub title: String,
    pub last_modified: Option<String>,
}

/// Raw page content: HTML plus OneNote ink if the page has any.
#[derive(Debug, Clone)]
pub struct PageContent {
    pub html: String,
    pub inkml: Option<String>,
}

pub struct OneNoteClient {
    agent: ureq::Agent,
    cfg: AuthConfig,
    tokens: Option<Tokens>,
    cache: Option<PathBuf>,
}

fn is_transient(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// OneNote error 20280 on the notebook list means the account has no OneNote
/// storage yet (it has never opened OneNote).
fn not_provisioned(e: Error) -> Error {
    match e {
        Error::Status { status: 500, ref body } if body.contains("\"20280\"") => Error::NotProvisioned,
        e => e,
    }
}

fn str_field(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_string()
}

fn section_of(v: &Value) -> RemoteSection {
    RemoteSection { id: str_field(v, "id"), name: str_field(v, "displayName") }
}

fn page_of(v: &Value) -> RemotePage {
    RemotePage {
        id: str_field(v, "id"),
        title: str_field(v, "title"),
        last_modified: v["lastModifiedDateTime"].as_str().map(str::to_string),
    }
}

impl OneNoteClient {
    /// `cache` is where tokens are persisted between runs (optional).
    pub fn new(cfg: AuthConfig, cache: Option<PathBuf>) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(120)))
            .build()
            .into();
        let tokens = cache
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Tokens>(&b).ok());
        Self { agent, cfg, tokens, cache }
    }

    pub fn config(&self) -> &AuthConfig {
        &self.cfg
    }

    pub fn is_signed_in(&self) -> bool {
        self.tokens.is_some()
    }

    pub fn begin_sign_in(&self) -> Result<DeviceCode> {
        auth::start_device_flow(&self.agent, &self.cfg)
    }

    pub fn complete_sign_in(&mut self, dc: &DeviceCode, cancel: &AtomicBool) -> Result<()> {
        let t = auth::poll_device_flow(&self.agent, &self.cfg, dc, cancel)?;
        self.set_tokens(Some(t));
        Ok(())
    }

    pub fn sign_out(&mut self) {
        self.set_tokens(None);
    }

    fn set_tokens(&mut self, t: Option<Tokens>) {
        if let Some(path) = &self.cache {
            match &t {
                Some(t) => {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    if let Err(e) = std::fs::write(path, serde_json::to_vec(t).unwrap_or_default()) {
                        log::warn!("could not cache tokens: {e}");
                    }
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        self.tokens = t;
    }

    fn bearer(&mut self) -> Result<String> {
        let t = self.tokens.as_ref().ok_or(Error::NotSignedIn)?;
        if t.is_fresh() {
            return Ok(format!("Bearer {}", t.access_token));
        }
        let Some(rt) = t.refresh_token.clone() else {
            self.set_tokens(None);
            return Err(Error::NotSignedIn);
        };
        match auth::refresh(&self.agent, &self.cfg, &rt) {
            Ok(t) => {
                let b = format!("Bearer {}", t.access_token);
                self.set_tokens(Some(t));
                Ok(b)
            }
            Err(e) => {
                self.set_tokens(None);
                Err(e)
            }
        }
    }

    fn check(resp: &mut ureq::http::Response<ureq::Body>) -> Result<()> {
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(());
        }
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        Err(Error::Status { status, body: body.chars().take(600).collect() })
    }

    /// GET with up to three retries on throttling and transient server errors.
    fn get_json(&mut self, url: &str) -> Result<Value> {
        let mut attempt = 0;
        loop {
            let auth = self.bearer()?;
            let mut resp = self.agent.get(url).header("Authorization", &auth).call()?;
            match Self::check(&mut resp) {
                Ok(()) => return resp.body_mut().read_json::<Value>().map_err(|e| Error::Parse(e.to_string())),
                Err(Error::Status { status, body }) if is_transient(status) && attempt < 3 => {
                    attempt += 1;
                    log::warn!("Graph returned {status}, retrying ({attempt}/3): {body}");
                    std::thread::sleep(Duration::from_millis(800 * (1 << attempt)));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// GETs a collection, following `@odata.nextLink`.
    fn get_all(&mut self, url: &str) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut next = Some(url.to_string());
        while let Some(url) = next.take() {
            let v = self.get_json(&url)?;
            if let Some(items) = v["value"].as_array() {
                out.extend(items.iter().cloned());
            }
            next = v["@odata.nextLink"].as_str().map(str::to_string);
        }
        Ok(out)
    }

    /// Notebooks with their sections (sections inside section groups are
    /// listed as "Group / Section").
    pub fn list_notebooks(&mut self) -> Result<Vec<RemoteNotebook>> {
        let url = format!(
            "{GRAPH_V1}/notebooks?$select=id,displayName&$expand=sections($select=id,displayName),sectionGroups($expand=sections($select=id,displayName))"
        );
        Ok(self
            .get_all(&url)
            .map_err(not_provisioned)?
            .iter()
            .map(|nb| {
                let mut sections: Vec<RemoteSection> =
                    nb["sections"].as_array().into_iter().flatten().map(section_of).collect();
                for g in nb["sectionGroups"].as_array().into_iter().flatten() {
                    let group = str_field(g, "displayName");
                    for s in g["sections"].as_array().into_iter().flatten() {
                        let mut s = section_of(s);
                        s.name = format!("{group} / {}", s.name);
                        sections.push(s);
                    }
                }
                RemoteNotebook { id: str_field(nb, "id"), name: str_field(nb, "displayName"), sections }
            })
            .collect())
    }

    pub fn list_pages(&mut self, section_id: &str) -> Result<Vec<RemotePage>> {
        let url = format!("{GRAPH_V1}/sections/{section_id}/pages?$select=id,title,lastModifiedDateTime&$top=100");
        Ok(self.get_all(&url)?.iter().map(page_of).collect())
    }

    fn post_json(&mut self, url: &str, body: Value) -> Result<Value> {
        let auth = self.bearer()?;
        let mut resp = self.agent.post(url).header("Authorization", &auth).send_json(body)?;
        Self::check(&mut resp)?;
        resp.body_mut().read_json::<Value>().map_err(|e| Error::Parse(e.to_string()))
    }

    pub fn create_notebook(&mut self, name: &str) -> Result<RemoteNotebook> {
        let v = self.post_json(&format!("{GRAPH_V1}/notebooks"), serde_json::json!({ "displayName": name }))?;
        Ok(RemoteNotebook { id: str_field(&v, "id"), name: str_field(&v, "displayName"), sections: Vec::new() })
    }

    pub fn create_section(&mut self, notebook_id: &str, name: &str) -> Result<RemoteSection> {
        let v = self.post_json(
            &format!("{GRAPH_V1}/notebooks/{notebook_id}/sections"),
            serde_json::json!({ "displayName": name }),
        )?;
        Ok(section_of(&v))
    }

    pub fn get_page(&mut self, page_id: &str) -> Result<RemotePage> {
        let v = self.get_json(&format!("{GRAPH_V1}/pages/{page_id}?$select=id,title,lastModifiedDateTime"))?;
        Ok(page_of(&v))
    }

    /// Page HTML (with generated element IDs) and its ink as InkML.
    pub fn get_page_content(&mut self, page_id: &str) -> Result<PageContent> {
        let auth = self.bearer()?;
        let url = format!("{GRAPH_BETA}/pages/{page_id}/content?includeIDs=true&includeInkML=true");
        let mut resp = self.agent.get(&url).header("Authorization", &auth).call()?;
        Self::check(&mut resp)?;
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = resp
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .map_err(|e| Error::Http(e.to_string()))?;
        if !ctype.to_ascii_lowercase().starts_with("multipart/") {
            return Ok(PageContent { html: body, inkml: None });
        }
        let boundary = multipart::boundary(&ctype).ok_or_else(|| Error::Parse("multipart without boundary".into()))?;
        let parts = multipart::decode(&body, &boundary);
        let html = parts.iter().find(|p| p.content_type.contains("html")).map(|p| p.body.clone()).unwrap_or_default();
        let inkml = parts.iter().find(|p| p.content_type.contains("inkml")).map(|p| p.body.clone());
        Ok(PageContent { html, inkml })
    }

    /// Downloads an image/file resource referenced by page HTML.
    pub fn get_resource(&mut self, url: &str) -> Result<Vec<u8>> {
        let auth = self.bearer()?;
        let mut resp = self.agent.get(url).header("Authorization", &auth).call()?;
        Self::check(&mut resp)?;
        resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| Error::Http(e.to_string()))
    }

    /// Creates a page from multipart parts (first part must be the HTML "presentation").
    pub fn create_page(&mut self, section_id: &str, parts: &[Part]) -> Result<RemotePage> {
        let auth = self.bearer()?;
        let (ctype, body) = multipart::encode(parts);
        let mut resp = self
            .agent
            .post(&format!("{GRAPH_BETA}/sections/{section_id}/pages"))
            .header("Authorization", &auth)
            .header("Content-Type", &ctype)
            .send(&body[..])?;
        Self::check(&mut resp)?;
        let v: Value = resp.body_mut().read_json().map_err(|e| Error::Parse(e.to_string()))?;
        Ok(page_of(&v))
    }

    /// Applies PATCH commands; `extra` holds binary parts referenced as `name:<part>`.
    pub fn patch_page(&mut self, page_id: &str, commands: &Value, extra: &[Part]) -> Result<()> {
        let auth = self.bearer()?;
        let commands = serde_json::to_vec(commands).unwrap_or_default();
        let mut parts = vec![Part { name: "Commands", content_type: "application/json", body: &commands }];
        parts.extend(extra.iter().map(|p| Part { name: p.name, content_type: p.content_type, body: p.body }));
        let (ctype, body) = multipart::encode(&parts);
        let mut resp = self
            .agent
            .patch(&format!("{GRAPH_V1}/pages/{page_id}/content"))
            .header("Authorization", &auth)
            .header("Content-Type", &ctype)
            .send(&body[..])?;
        Self::check(&mut resp)
    }

    pub fn delete_page(&mut self, page_id: &str) -> Result<()> {
        let auth = self.bearer()?;
        let mut resp = self.agent.delete(&format!("{GRAPH_V1}/pages/{page_id}")).header("Authorization", &auth).call()?;
        match Self::check(&mut resp) {
            Err(Error::Status { status: 404, .. }) => Ok(()),
            r => r,
        }
    }
}
