//! Background worker that owns the OneNote client, so network calls never
//! block drawing. The UI sends [`Job`]s and polls [`Event`]s each frame.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use onenote::convert::{export_page, import_page};
use onenote::{AuthConfig, DeviceCode, ExportMode, ImportSource, OneNoteClient, RemoteNotebook, RemotePage, RemoteSection};
use sketch_core::{Id, OneNotePageLink, OneNoteSectionLink, Page};

pub enum Job {
    Configure(AuthConfig),
    SignIn,
    SignOut,
    ListNotebooks,
    ListPages { section_id: String },
    CreateNotebook { name: String },
    ImportPage { remote_section: String, page: RemotePage, local_section: Id },
    ImportSection { remote_section: String, local_section: Id },
    ExportPage { remote_section: String, page: Box<Page>, mode: ExportMode, replace_old: bool },
    ExportSection {
        notebook_id: String,
        /// Existing linked OneNote section, or `None` to create one named `name`.
        remote_section: Option<String>,
        name: String,
        local_section: Id,
        pages: Vec<Page>,
        mode: ExportMode,
        replace_old: bool,
    },
}

pub enum Event {
    DeviceCode(DeviceCode),
    SignedIn,
    SignedOut,
    Notebooks(Vec<RemoteNotebook>),
    Pages { section_id: String, pages: Vec<RemotePage> },
    NotebookCreated(RemoteNotebook),
    Imported { local_section: Id, page: Box<Page>, source: ImportSource },
    Exported { page: Id, link: OneNotePageLink },
    SectionLinked { local_section: Id, link: OneNoteSectionLink, remote: RemoteSection },
    Info(String),
    Error(String),
    Idle,
}

pub struct SyncWorker {
    jobs: Sender<Job>,
    events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    pub busy: bool,
    pub signed_in: bool,
}

impl SyncWorker {
    pub fn spawn(cfg: AuthConfig, token_cache: std::path::PathBuf, ctx: egui::Context) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (ev_tx, events) = channel::<Event>();
        let cancel = Arc::new(AtomicBool::new(false));
        let client = OneNoteClient::new(cfg, Some(token_cache.clone()));
        let signed_in = client.is_signed_in();
        let worker_cancel = cancel.clone();
        std::thread::Builder::new()
            .name("onenote-sync".into())
            .spawn(move || {
                let mut client = client;
                let send = |e: Event| {
                    let _ = ev_tx.send(e);
                    ctx.request_repaint();
                };
                while let Ok(job) = job_rx.recv() {
                    if let Err(e) = run(&mut client, job, &send, &worker_cancel, &token_cache) {
                        if matches!(e, onenote::Error::NotSignedIn) {
                            send(Event::SignedOut);
                        }
                        send(Event::Error(e.to_string()));
                    }
                    send(Event::Idle);
                }
            })
            .expect("spawn sync thread");
        Self { jobs, events, cancel, busy: false, signed_in }
    }

    pub fn send(&mut self, job: Job) {
        self.busy = true;
        self.cancel.store(false, Ordering::Relaxed);
        let _ = self.jobs.send(job);
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn poll(&mut self) -> Vec<Event> {
        let events: Vec<Event> = self.events.try_iter().collect();
        for e in &events {
            match e {
                Event::Idle => self.busy = false,
                Event::SignedIn => self.signed_in = true,
                Event::SignedOut => self.signed_in = false,
                _ => {}
            }
        }
        events
    }
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
        Job::CreateNotebook { name } => {
            send(Event::NotebookCreated(client.create_notebook(&name)?));
            send(Event::Notebooks(client.list_notebooks()?));
        }
        Job::ImportPage { remote_section, page, local_section } => {
            send(Event::Info(format!("Opening \"{}\" from OneNote…", page.title)));
            let imported = import_page(client, &remote_section, &page)?;
            send(Event::Imported { local_section, page: Box::new(imported.page), source: imported.source });
        }
        Job::ImportSection { remote_section, local_section } => {
            let pages = client.list_pages(&remote_section)?;
            let total = pages.len();
            for (i, page) in pages.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                send(Event::Info(format!("Importing {}/{total}: {}", i + 1, page.title)));
                match import_page(client, &remote_section, page) {
                    Ok(imp) => send(Event::Imported { local_section, page: Box::new(imp.page), source: imp.source }),
                    Err(e) => send(Event::Error(format!("\"{}\": {e}", page.title))),
                }
            }
            send(Event::Info(format!("Imported {total} page(s)")));
        }
        Job::ExportPage { remote_section, page, mode, replace_old } => {
            send(Event::Info(format!("Saving \"{}\" to OneNote…", page.title)));
            let link = export_page(client, &remote_section, &page, mode, replace_old)?;
            send(Event::Exported { page: page.id, link });
            send(Event::Info(format!("Saved \"{}\" to OneNote", page.title)));
        }
        Job::ExportSection { notebook_id, remote_section, name, local_section, pages, mode, replace_old } => {
            let section = match remote_section {
                Some(id) => RemoteSection { id, name: name.clone() },
                None => client.create_section(&notebook_id, &name)?,
            };
            send(Event::SectionLinked {
                local_section,
                link: OneNoteSectionLink { notebook_id, section_id: section.id.clone() },
                remote: section.clone(),
            });
            let total = pages.len();
            for (i, page) in pages.into_iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                send(Event::Info(format!("Uploading {}/{total}: {}", i + 1, page.title)));
                match export_page(client, &section.id, &page, mode, replace_old) {
                    Ok(link) => send(Event::Exported { page: page.id, link }),
                    Err(e) => send(Event::Error(format!("\"{}\": {e}", page.title))),
                }
            }
            send(Event::Info(format!("Section \"{name}\" saved to OneNote ({total} page(s))")));
        }
    }
    Ok(())
}
