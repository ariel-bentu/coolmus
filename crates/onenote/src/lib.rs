//! OneNote integration via Microsoft Graph.
//!
//! This crate talks to OneNote's cloud API only — it never automates the
//! OneNote desktop application.
//!
//! * [`auth`]     — OAuth device-code sign-in with token refresh/caching.
//! * [`client`]   — browse notebooks/sections/pages, create/patch/delete pages.
//! * [`inkml`]    — InkML ⇄ Sketcher strokes (OneNote's native ink format).
//! * [`convert`]  — Sketcher page ⇄ OneNote page (HTML + ink + data attachment).

pub mod auth;
pub mod client;
pub mod convert;
pub mod html;
pub mod inkml;
pub mod multipart;

pub use auth::{AuthConfig, DeviceCode, Tokens};
pub use client::{OneNoteClient, RemoteNotebook, RemotePage, RemoteSection};
pub use convert::{ExportMode, ImportSource, ImportedPage};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(String),
    #[error("OneNote returned HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("sign-in failed: {0}")]
    Auth(String),
    #[error("not signed in")]
    NotSignedIn,
    /// OneNote storage doesn't exist yet for this account (never used OneNote).
    /// The text is a UI translation key.
    #[error("oneNoteNotProvisioned")]
    NotProvisioned,
    #[error("could not parse OneNote data: {0}")]
    Parse(String),
    #[error("render error: {0}")]
    Render(String),
}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Error::Http(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
