//! Background worker that owns the OneNote client, so network calls never
//! block the UI. Jobs go in through a channel; results come back through a callback.
//!
//! OneNote is only a source: pages are imported into local folders and never
//! written back.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;

use onenote::convert::import_page;
use onenote::{AuthConfig, DeviceCode, ImportSource, OneNoteClient, RemoteNotebook, RemotePage, RemoteSection};
use sketch_core::Page;

pub enum Job {
    Configure(AuthConfig),
    SignIn,
    SignOut,
    ListNotebooks,
    ListPages { section_id: String },
    /// One page into `folder`; `open` asks the UI to open it afterwards.
    ImportPage { section_id: String, page: RemotePage, folder: String, open: bool },
    /// Every page of a section into `folder/<section name>`.
    ImportSection { section: RemoteSection, folder: String },
    /// Every section into `folder/<notebook name>/<section name>`.
    ImportNotebook { notebook: RemoteNotebook, folder: String },
}

pub enum Event {
    Busy(bool),
    DeviceCode(DeviceCode),
    SignedIn,
    SignedOut,
    Notebooks(Vec<RemoteNotebook>),
    Pages { section_id: String, pages: Vec<RemotePage> },
    Imported { folder: String, page: Box<Page>, source: ImportSource, open: bool },
    /// A translatable progress message: key plus positional arguments.
    Info { key: &'static str, args: Vec<String> },
    Error(String),
}

pub struct SyncHandle {
    jobs: Sender<Job>,
    cancel: Arc<AtomicBool>,
}

impl SyncHandle {
    /// Starts the worker. Returns the handle and whether cached tokens exist.
    pub fn spawn(
        cfg: AuthConfig,
        token_cache: std::path::PathBuf,
        on_event: impl Fn(Event) + Send + 'static,
    ) -> (Self, bool) {
        let (jobs, job_rx) = channel::<Job>();
        let cancel = Arc::new(AtomicBool::new(false));
        let client = OneNoteClient::new(cfg, Some(token_cache.clone()));
        let signed_in = client.is_signed_in();
        let worker_cancel = cancel.clone();
        std::thread::Builder::new()
            .name("onenote-sync".into())
            .spawn(move || {
                let mut client = client;
                while let Ok(job) = job_rx.recv() {
                    worker_cancel.store(false, Ordering::Relaxed);
                    on_event(Event::Busy(true));
                    if let Err(e) = run(&mut client, job, &on_event, &worker_cancel, &token_cache) {
                        if matches!(e, onenote::Error::NotSignedIn) {
                            on_event(Event::SignedOut);
                        }
                        on_event(Event::Error(e.to_string()));
                    }
                    on_event(Event::Busy(false));
                }
            })
            .expect("spawn sync thread");
        (Self { jobs, cancel }, signed_in)
    }

    pub fn send(&self, job: Job) {
        let _ = self.jobs.send(job);
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn info(key: &'static str, args: &[&dyn ToString]) -> Event {
    Event::Info { key, args: args.iter().map(|a| a.to_string()).collect() }
}

/// `parent/name`, with `name` made safe for the file system.
fn subfolder(parent: &str, name: &str) -> String {
    let name = sketch_store::sanitize_name(name).unwrap_or_else(|_| "OneNote".into());
    if parent.is_empty() { name } else { format!("{parent}/{name}") }
}

/// Imports every page of `section` into `folder`. Returns the number of pages seen.
fn import_section(
    client: &mut OneNoteClient,
    section: &RemoteSection,
    folder: &str,
    send: &dyn Fn(Event),
    cancel: &AtomicBool,
) -> onenote::Result<usize> {
    let pages = client.list_pages(&section.id)?;
    let total = pages.len();
    for (i, page) in pages.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        send(info("importing", &[&(i + 1), &total, &page.title]));
        match import_page(client, &section.id, page) {
            Ok(imp) => send(Event::Imported {
                folder: folder.to_string(),
                page: Box::new(imp.page),
                source: imp.source,
                open: false,
            }),
            Err(e) => send(Event::Error(format!("\"{}\": {e}", page.title))),
        }
    }
    Ok(total)
}

fn run(
    client: &mut OneNoteClient,
    job: Job,
    send: &dyn Fn(Event),
    cancel: &AtomicBool,
    token_cache: &std::path::Path,
) -> onenote::Result<()> {
    match job {
        Job::Configure(cfg) => {
            if &cfg != client.config() {
                *client = OneNoteClient::new(cfg, Some(token_cache.to_path_buf()));
                client.sign_out();
                send(Event::SignedOut);
            }
        }
        Job::SignIn => {
            let dc = client.begin_sign_in()?;
            send(Event::DeviceCode(dc.clone()));
            client.complete_sign_in(&dc, cancel)?;
            send(Event::SignedIn);
            send(Event::Notebooks(client.list_notebooks()?));
        }
        Job::SignOut => {
            client.sign_out();
            send(Event::SignedOut);
        }
        Job::ListNotebooks => send(Event::Notebooks(client.list_notebooks()?)),
        Job::ListPages { section_id } => {
            let pages = client.list_pages(&section_id)?;
            send(Event::Pages { section_id, pages });
        }
        Job::ImportPage { section_id, page, folder, open } => {
            send(info("opening", &[&page.title]));
            let imported = import_page(client, &section_id, &page)?;
            send(Event::Imported { folder, page: Box::new(imported.page), source: imported.source, open });
        }
        Job::ImportSection { section, folder } => {
            let target = subfolder(&folder, &section.name);
            let total = import_section(client, &section, &target, send, cancel)?;
            send(info("importedN", &[&total, &target]));
        }
        Job::ImportNotebook { notebook, folder } => {
            let base = subfolder(&folder, &notebook.name);
            let mut total = 0;
            for section in &notebook.sections {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                total += import_section(client, section, &subfolder(&base, &section.name), send, cancel)?;
            }
            send(info("importedN", &[&total, &base]));
        }
    }
    Ok(())
}
