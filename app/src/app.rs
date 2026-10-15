//! Application state: the open page, library operations, and OneNote events.

use onenote::{DeviceCode, ImportSource, RemoteNotebook, RemotePage};
use sketch_core::history::{Edit, History};
use sketch_core::{Id, Notebook, OneNoteSectionLink, Page, PageLocation, Pen, Section};
use sketch_store::Store;

use crate::canvas::{Canvas, Tool};
use crate::settings::{config_dir, Settings};
use crate::sync::{Event, Job, SyncWorker};

pub struct OpenPage {
    pub page: Page,
    pub section: Id,
    pub history: History,
    pub dirty: bool,
    pub last_change: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Notebook(Id),
    Section(Id),
    Page(Id),
}

pub enum Dialog {
    Rename { target: Target, text: String },
    ConfirmDelete { target: Target, label: String },
}

#[derive(Default)]
pub struct OneNoteState {
    pub open: bool,
    pub device_code: Option<DeviceCode>,
    pub notebooks: Vec<RemoteNotebook>,
    pub selected_notebook: Option<String>,
    /// (section id, section name, notebook id)
    pub selected_section: Option<(String, String, String)>,
    pub pages: Vec<RemotePage>,
    pub new_notebook_name: String,
    pub client_id_edit: String,
    pub tenant_edit: String,
    /// Remote page to open as soon as it is imported.
    pub pending_open: Option<String>,
}

pub struct SketcherApp {
    pub store: Store,
    pub settings: Settings,
    pub current: Option<OpenPage>,
    pub canvas: Canvas,
    pub tool: Tool,
    pub search: String,
    pub tag_input: String,
    pub dialog: Option<Dialog>,
    pub onenote: OneNoteState,
    pub sync: SyncWorker,
    pub status: String,
    pub errors: Vec<String>,
    pub show_settings: bool,
    pub pending_onenote_save: bool,
}

impl SketcherApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let store = Store::open(sketch_store::default_root()).unwrap_or_else(|e| {
            log::error!("could not open library: {e}; using a temporary one");
            Store::open(std::env::temp_dir().join("sketcher-fallback")).expect("temp store")
        });
        let settings = Settings::load();
        let sync = SyncWorker::spawn(settings.auth.clone(), config_dir().join("tokens.json"), cc.egui_ctx.clone());
        let mut canvas = Canvas::default();
        canvas.smoothing = settings.smoothing;
        let mut app = Self {
            onenote: OneNoteState {
                client_id_edit: settings.auth.client_id.clone(),
                tenant_edit: settings.auth.tenant.clone(),
                ..Default::default()
            },
            store,
            settings,
            current: None,
            canvas,
            tool: Tool::Pen,
            search: String::new(),
            tag_input: String::new(),
            dialog: None,
            sync,
            status: String::new(),
            errors: Vec::new(),
            show_settings: false,
            pending_onenote_save: false,
        };
        // Open the most recent page, or create one.
        match app.store.search("").first().map(|(loc, _)| *loc) {
            Some(loc) => app.open_page(loc),
            None => {
                let section = app.store.library.notebooks[0].sections[0].id;
                app.new_page(section);
            }
        }
        app
    }

    pub fn pen(&self) -> Pen {
        self.settings.pen
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        log::error!("{msg}");
        self.errors.push(msg);
    }

    // ---------- page lifecycle ----------

    pub fn save_current(&mut self) {
        let Some(cur) = self.current.as_mut() else { return };
        if !cur.dirty {
            return;
        }
        match self.store.save_page(cur.section, &cur.page) {
            Ok(()) => cur.dirty = false,
            Err(e) => {
                let msg = format!("Save failed: {e}");
                self.error(msg);
            }
        }
    }

    pub fn mark_dirty(&mut self, now: f64) {
        if let Some(cur) = self.current.as_mut() {
            cur.dirty = true;
            cur.last_change = now;
            cur.page.touch();
        }
    }

    pub fn open_page(&mut self, loc: PageLocation) {
        if self.current.as_ref().is_some_and(|c| c.page.id == loc.page) {
            return;
        }
        self.save_current();
        match self.store.load_page(loc.page) {
            Ok(page) => {
                self.current = Some(OpenPage { page, section: loc.section, history: History::default(), dirty: false, last_change: 0.0 });
                self.canvas.reset_for_new_page();
            }
            Err(e) => self.error(format!("Could not open page: {e}")),
        }
    }

    pub fn new_page(&mut self, section: Id) {
        self.save_current();
        let n = self.store.library.section(section).map_or(0, |s| s.pages.len()) + 1;
        let mut page = Page::new(format!("Page {n}"));
        if let Some(cur) = &self.current {
            page.paper = cur.page.paper;
        }
        if let Err(e) = self.store.save_page(section, &page) {
            self.error(format!("Could not create page: {e}"));
            return;
        }
        self.current = Some(OpenPage { page, section, history: History::default(), dirty: false, last_change: 0.0 });
        self.canvas.reset_for_new_page();
    }

    pub fn new_section(&mut self, notebook: Id) -> Option<Id> {
        let nb = self.store.library.notebook_mut(notebook)?;
        let section = Section::new(format!("Section {}", nb.sections.len() + 1));
        let id = section.id;
        nb.sections.push(section);
        self.persist_library();
        Some(id)
    }

    pub fn new_notebook(&mut self) {
        let mut nb = Notebook::new(format!("Notebook {}", self.store.library.notebooks.len() + 1));
        nb.sections.push(Section::new("Section 1"));
        self.store.library.notebooks.push(nb);
        self.persist_library();
    }

    pub fn persist_library(&mut self) {
        if let Err(e) = self.store.save_library() {
            self.error(format!("Could not save library: {e}"));
        }
    }

    pub fn rename(&mut self, target: Target, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        match target {
            Target::Notebook(id) => {
                if let Some(nb) = self.store.library.notebook_mut(id) {
                    nb.name = name;
                }
                self.persist_library();
            }
            Target::Section(id) => {
                if let Some(s) = self.store.library.section_mut(id) {
                    s.name = name;
                }
                self.persist_library();
            }
            Target::Page(id) => {
                if let Some(cur) = self.current.as_mut().filter(|c| c.page.id == id) {
                    cur.page.title = name;
                    cur.dirty = true;
                    self.save_current();
                } else if let Some(loc) = self.store.library.locate_page(id) {
                    match self.store.load_page(id) {
                        Ok(mut p) => {
                            p.title = name;
                            p.touch();
                            if let Err(e) = self.store.save_page(loc.section, &p) {
                                self.error(e.to_string());
                            }
                        }
                        Err(e) => self.error(e.to_string()),
                    }
                }
            }
        }
    }

    pub fn delete(&mut self, target: Target) {
        let current_loc = self.current.as_ref().and_then(|c| self.store.library.locate_page(c.page.id));
        let closes_current = current_loc.is_some_and(|loc| match target {
            Target::Notebook(id) => loc.notebook == id,
            Target::Section(id) => loc.section == id,
            Target::Page(id) => loc.page == id,
        });
        if closes_current {
            self.current = None;
        }
        let result = match target {
            Target::Notebook(id) => self.store.delete_notebook(id),
            Target::Section(id) => self.store.delete_section(id),
            Target::Page(id) => self.store.delete_page(id),
        };
        if let Err(e) = result {
            self.error(format!("Delete failed: {e}"));
        }
        if self.store.library.notebooks.is_empty() {
            self.new_notebook();
        }
        if self.current.is_none() {
            match self.store.search("").first().map(|(l, _)| *l) {
                Some(loc) => self.open_page(loc),
                None => {
                    let nb = &self.store.library.notebooks[0];
                    let sec = match nb.sections.first() {
                        Some(s) => s.id,
                        None => {
                            let nb_id = nb.id;
                            self.new_section(nb_id).expect("notebook exists")
                        }
                    };
                    self.new_page(sec);
                }
            }
        }
    }

    // ---------- editing ----------

    pub fn apply_edit(&mut self, edit: Edit, now: f64) {
        if let Some(cur) = self.current.as_mut() {
            cur.history.apply(&mut cur.page, edit);
            cur.dirty = true;
            cur.last_change = now;
            self.canvas.invalidate();
        }
    }

    pub fn undo(&mut self, now: f64) {
        if let Some(cur) = self.current.as_mut() {
            if cur.history.undo(&mut cur.page) {
                cur.dirty = true;
                cur.last_change = now;
                self.canvas.invalidate();
            }
        }
    }

    pub fn redo(&mut self, now: f64) {
        if let Some(cur) = self.current.as_mut() {
            if cur.history.redo(&mut cur.page) {
                cur.dirty = true;
                cur.last_change = now;
                self.canvas.invalidate();
            }
        }
    }

    pub fn clear_page(&mut self, now: f64) {
        let Some(cur) = self.current.as_ref() else { return };
        if cur.page.strokes.is_empty() {
            return;
        }
        let before = cur.page.strokes.clone();
        self.apply_edit(Edit::Replace { before, after: Vec::new() }, now);
    }

    pub fn export_png(&mut self) {
        let Some(cur) = self.current.as_ref() else { return };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(format!("{}.png", cur.page.title))
            .save_file()
        else {
            return;
        };
        let opts = sketch_render::raster::RasterOptions { scale: 2.0, ..Default::default() };
        match sketch_render::raster::render_png(&cur.page, opts).map_err(|e| e.to_string()).and_then(|png| std::fs::write(&path, png).map_err(|e| e.to_string())) {
            Ok(()) => self.status = format!("Exported {}", path.display()),
            Err(e) => self.error(format!("Export failed: {e}")),
        }
    }

    // ---------- OneNote ----------

    /// Local section mirroring a OneNote section (created on demand inside a
    /// local notebook named after the OneNote notebook).
    pub fn ensure_local_section(&mut self, remote_nb: &str, remote_section: &str, section_name: &str) -> Id {
        if let Some((_, s)) = self.store.library.find_by_onenote_section(remote_section) {
            return s;
        }
        let nb_name = self
            .onenote
            .notebooks
            .iter()
            .find(|n| n.id == remote_nb)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "OneNote".into());
        let local_nb_name = format!("{nb_name} (OneNote)");
        let nb_id = match self.store.library.notebooks.iter().find(|n| n.name == local_nb_name) {
            Some(nb) => nb.id,
            None => {
                let nb = Notebook::new(local_nb_name);
                let id = nb.id;
                self.store.library.notebooks.push(nb);
                id
            }
        };
        let mut section = Section::new(section_name);
        section.onenote = Some(OneNoteSectionLink { notebook_id: remote_nb.to_string(), section_id: remote_section.to_string() });
        let id = section.id;
        self.store.library.notebook_mut(nb_id).expect("just ensured").sections.push(section);
        self.persist_library();
        id
    }

    /// Remote section the current page should be saved to.
    pub fn export_target_for_current(&self) -> Option<String> {
        let cur = self.current.as_ref()?;
        cur.page
            .onenote
            .as_ref()
            .map(|l| l.section_id.clone())
            .or_else(|| self.store.library.section(cur.section)?.onenote.as_ref().map(|l| l.section_id.clone()))
            .or_else(|| self.onenote.selected_section.as_ref().map(|(id, _, _)| id.clone()))
    }

    pub fn save_current_to_onenote(&mut self) {
        if !self.sync.signed_in {
            self.onenote.open = true;
            self.error("Sign in to OneNote first.");
            return;
        }
        let Some(target) = self.export_target_for_current() else {
            self.onenote.open = true;
            self.error("Choose a OneNote section in the OneNote panel first.");
            return;
        };
        if let Some(cur) = self.current.as_mut() {
            cur.dirty = true;
        }
        self.save_current();
        let Some(cur) = self.current.as_ref() else { return };
        let job = Job::ExportPage {
            remote_section: target,
            page: Box::new(cur.page.clone()),
            mode: self.settings.export_mode,
            replace_old: self.settings.replace_old,
        };
        self.sync.send(job);
    }

    pub fn push_section_to_onenote(&mut self, section: Id) {
        if !self.sync.signed_in {
            self.onenote.open = true;
            self.error("Sign in to OneNote first.");
            return;
        }
        self.save_current();
        let Some(sec) = self.store.library.section(section).cloned() else { return };
        let (notebook_id, remote_section) = match &sec.onenote {
            Some(link) => (link.notebook_id.clone(), Some(link.section_id.clone())),
            None => match &self.onenote.selected_notebook {
                Some(nb) => (nb.clone(), None),
                None => {
                    self.onenote.open = true;
                    self.error("Choose a OneNote notebook in the OneNote panel first.");
                    return;
                }
            },
        };
        let mut pages = Vec::new();
        for meta in &sec.pages {
            match self.store.load_page(meta.id) {
                Ok(p) => pages.push(p),
                Err(e) => self.error(format!("Skipping \"{}\": {e}", meta.title)),
            }
        }
        self.sync.send(Job::ExportSection {
            notebook_id,
            remote_section,
            name: sec.name.clone(),
            local_section: section,
            pages,
            mode: self.settings.export_mode,
            replace_old: self.settings.replace_old,
        });
    }

    pub fn handle_sync_events(&mut self) {
        for ev in self.sync.poll() {
            match ev {
                Event::DeviceCode(dc) => {
                    self.onenote.device_code = Some(dc);
                    self.onenote.open = true;
                }
                Event::SignedIn => {
                    self.onenote.device_code = None;
                    self.status = "Signed in to OneNote".into();
                }
                Event::SignedOut => {
                    self.onenote.device_code = None;
                    self.onenote.notebooks.clear();
                    self.onenote.pages.clear();
                    self.onenote.selected_section = None;
                    self.onenote.selected_notebook = None;
                }
                Event::Notebooks(nbs) => self.onenote.notebooks = nbs,
                Event::Pages { section_id, pages } => {
                    if self.onenote.selected_section.as_ref().is_some_and(|(id, _, _)| *id == section_id) {
                        self.onenote.pages = pages;
                    }
                }
                Event::NotebookCreated(nb) => {
                    self.status = format!("Created OneNote notebook \"{}\"", nb.name);
                    self.onenote.selected_notebook = Some(nb.id);
                }
                Event::Imported { local_section, page, source } => self.on_imported(local_section, *page, source),
                Event::Exported { page, link } => self.on_exported(page, link),
                Event::SectionLinked { local_section, link, remote } => {
                    if let Some(s) = self.store.library.section_mut(local_section) {
                        s.onenote = Some(link.clone());
                    }
                    self.persist_library();
                    if let Some(nb) = self.onenote.notebooks.iter_mut().find(|n| n.id == link.notebook_id) {
                        if !nb.sections.iter().any(|s| s.id == remote.id) {
                            nb.sections.push(remote);
                        }
                    }
                }
                Event::Info(msg) => self.status = msg,
                Event::Error(msg) => {
                    self.onenote.device_code = None;
                    self.error(msg);
                }
                Event::Idle => {}
            }
        }
    }

    fn on_imported(&mut self, local_section: Id, mut page: Page, source: ImportSource) {
        let remote_id = page.onenote.as_ref().map(|l| l.page_id.clone());
        // Re-importing a page we already have refreshes it in place.
        let mut section = local_section;
        if let Some(loc) = remote_id.as_deref().and_then(|id| self.store.library.find_by_onenote_page(id)) {
            page.id = loc.page;
            section = loc.section;
        }
        if let Err(e) = self.store.save_page(section, &page) {
            self.error(format!("Could not store imported page: {e}"));
            return;
        }
        let is_current = self.current.as_ref().is_some_and(|c| c.page.id == page.id);
        let wanted = remote_id.is_some() && self.onenote.pending_open == remote_id;
        if is_current || wanted {
            self.onenote.pending_open = None;
            self.save_current();
            self.current = None;
            self.open_page(PageLocation { notebook: Id::nil(), section, page: page.id });
        }
        match source {
            ImportSource::Empty => self.status = format!("\"{}\" has no ink; text and images are not imported", page.title),
            ImportSource::NativeInk => self.status = format!("Opened \"{}\" (OneNote ink)", page.title),
            ImportSource::SketcherData => self.status = format!("Opened \"{}\"", page.title),
        }
    }

    fn on_exported(&mut self, page_id: Id, link: sketch_core::OneNotePageLink) {
        if let Some(cur) = self.current.as_mut().filter(|c| c.page.id == page_id) {
            cur.page.onenote = Some(link);
            cur.dirty = true;
            self.save_current();
            return;
        }
        let Some(loc) = self.store.library.locate_page(page_id) else { return };
        match self.store.load_page(page_id) {
            Ok(mut p) => {
                p.onenote = Some(link);
                if let Err(e) = self.store.save_page(loc.section, &p) {
                    self.error(e.to_string());
                }
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context, now: f64) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::{Key, KeyboardShortcut, Modifiers};
        let sc = |m, k| KeyboardShortcut::new(m, k);
        let (undo, redo, redo2, save, new, fit, pen, eraser, pan) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Z)),
                i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)),
                i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Y)),
                i.consume_shortcut(&sc(Modifiers::COMMAND, Key::S)),
                i.consume_shortcut(&sc(Modifiers::COMMAND, Key::N)),
                i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Num0)),
                i.consume_key(Modifiers::NONE, Key::P),
                i.consume_key(Modifiers::NONE, Key::E),
                i.consume_key(Modifiers::NONE, Key::H),
            )
        });
        if redo || redo2 {
            self.redo(now);
        } else if undo {
            self.undo(now);
        }
        if save {
            if let Some(c) = self.current.as_mut() {
                c.dirty = true;
            }
            self.save_current();
            self.status = "Saved".into();
        }
        if new {
            if let Some(sec) = self.current.as_ref().map(|c| c.section) {
                self.new_page(sec);
            }
        }
        if fit {
            self.canvas.request_fit();
        }
        if pen {
            self.tool = Tool::Pen;
        }
        if eraser {
            self.tool = Tool::Eraser;
        }
        if pan {
            self.tool = Tool::Pan;
        }
    }
}

impl eframe::App for SketcherApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.handle_sync_events();
        self.handle_shortcuts(&ctx, now);

        crate::ui_toolbar::show(self, ui, now);
        crate::ui_library::show(self, ui);
        crate::ui_toolbar::status_bar(self, ui);

        egui::CentralPanel::no_frame().show(ui, |ui| {
            let pen = self.pen();
            let tool = self.tool;
            if let Some(cur) = self.current.as_ref() {
                if let Some(edit) = self.canvas.show(ui, &cur.page, &pen, tool) {
                    self.apply_edit(edit, now);
                }
            } else {
                ui.centered_and_justified(|ui| ui.label("No page open"));
            }
        });

        crate::ui_onenote::show(self, &ctx);
        crate::ui_library::dialogs(self, &ctx);
        crate::ui_toolbar::settings_window(self, &ctx);
        crate::ui_toolbar::error_toasts(self, &ctx);

        // Autosave shortly after the last change, never mid-stroke.
        if let Some(cur) = &self.current {
            if cur.dirty && !self.canvas.is_drawing() {
                let wait = cur.last_change + 1.0 - now;
                if wait <= 0.0 {
                    self.save_current();
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
                }
            }
        }
    }

    fn on_exit(&mut self) {
        self.save_current();
        self.settings.save();
    }
}
