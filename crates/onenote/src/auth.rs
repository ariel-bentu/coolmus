//! OAuth 2.0 device-code flow against the Microsoft identity platform.
//!
//! Device code needs no redirect URI or embedded browser: the user opens
//! microsoft.com/devicelogin, types a short code and signs in. Requires an
//! Entra ID app registration with "Allow public client flows" enabled and the
//! delegated `Notes.ReadWrite` permission (see README).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

pub const SCOPES: &str = "Notes.ReadWrite offline_access";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthConfig {
    /// Application (client) ID of your Entra ID app registration.
    pub client_id: String,
    /// `common` (work + personal accounts), `consumers`, `organizations` or a tenant ID.
    pub tenant: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self { client_id: String::new(), tenant: "common".into() }
    }
}

impl AuthConfig {
    fn endpoint(&self, path: &str) -> String {
        format!("https://login.microsoftonline.com/{}/oauth2/v2.0/{path}", self.tenant)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub user_code: String,
    pub device_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: u64,
}

impl Tokens {
    pub fn is_fresh(&self) -> bool {
        unix_now() + 120 < self.expires_at
    }
}

fn unix_now() -> u64 {
    sketch_core::now_millis() / 1000
}

fn post_form(agent: &ureq::Agent, url: &str, form: &[(&str, &str)]) -> Result<Value> {
    let mut resp = agent.post(url).send_form(form.iter().copied())?;
    resp.body_mut().read_json::<Value>().map_err(|e| Error::Auth(e.to_string()))
}

fn tokens_from(v: &Value) -> Result<Tokens> {
    let access_token = v["access_token"]
        .as_str()
        .ok_or_else(|| Error::Auth(v["error_description"].as_str().unwrap_or("no access token").to_string()))?;
    Ok(Tokens {
        access_token: access_token.to_string(),
        refresh_token: v["refresh_token"].as_str().map(str::to_string),
        expires_at: unix_now() + v["expires_in"].as_u64().unwrap_or(3600),
    })
}

pub fn start_device_flow(agent: &ureq::Agent, cfg: &AuthConfig) -> Result<DeviceCode> {
    if cfg.client_id.trim().is_empty() {
        return Err(Error::Auth("no client ID configured (see Settings)".into()));
    }
    let v = post_form(agent, &cfg.endpoint("devicecode"), &[("client_id", &cfg.client_id), ("scope", SCOPES)])?;
    if let Some(err) = v["error_description"].as_str() {
        return Err(Error::Auth(err.to_string()));
    }
    serde_json::from_value(v).map_err(|e| Error::Auth(e.to_string()))
}

/// Blocks until the user finishes signing in, the code expires, or `cancel` is set.
pub fn poll_device_flow(agent: &ureq::Agent, cfg: &AuthConfig, dc: &DeviceCode, cancel: &AtomicBool) -> Result<Tokens> {
    let deadline = unix_now() + dc.expires_in;
    let mut interval = dc.interval.max(1);
    while unix_now() < deadline {
        for _ in 0..interval * 10 {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Auth("cancelled".into()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let v = post_form(
            agent,
            &cfg.endpoint("token"),
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", &cfg.client_id),
                ("device_code", &dc.device_code),
            ],
        )?;
        match v["error"].as_str() {
            None => return tokens_from(&v),
            Some("authorization_pending") => {}
            Some("slow_down") => interval += 5,
            Some(_) => {
                return Err(Error::Auth(v["error_description"].as_str().unwrap_or("sign-in failed").to_string()))
            }
        }
    }
    Err(Error::Auth("the sign-in code expired".into()))
}

pub fn refresh(agent: &ureq::Agent, cfg: &AuthConfig, refresh_token: &str) -> Result<Tokens> {
    let v = post_form(
        agent,
        &cfg.endpoint("token"),
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &cfg.client_id),
            ("refresh_token", refresh_token),
            ("scope", SCOPES),
        ],
    )?;
    let mut t = tokens_from(&v)?;
    // Microsoft may omit a new refresh token; keep the old one then.
    t.refresh_token.get_or_insert_with(|| refresh_token.to_string());
    Ok(t)
}
