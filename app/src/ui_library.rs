//! Left panel: search, tag cloud and the notebook → section → page tree.

use egui::RichText;
use sketch_core::{Id, PageLocation};

use crate::app::{Dialog, SketcherApp, Target};

enum Action {
    Open(PageLocation),
    NewPage(Id),
    NewSection(Id),
    NewNotebook,
    Rename(Target, String),
    Delete(Target, String),
    Push(Id),
}

pub fn show(app: &mut SketcherApp, ui: &mut egui::Ui) {
    let mut action = None;
    let current = app.current.as_ref().map(|c| c.page.id);
    egui::Panel::left("library").resizable(true).default_size(250.0).show(ui, |ui| {
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(&mut app.search).hint_text("🔍 Search title or #tag").desired_width(f32::INFINITY));
        let tags = app.store.library.all_tags();
        if !tags.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                for t in tags.iter().take(30) {
                    let token = format!("#{t}");
                    let active = app.search.split_whitespace().any(|w| w.eq_ignore_ascii_case(&token));
                    if ui.selectable_label(active, RichText::new(&token).small()).clicked() {
                        app.search = if active {
                            app.search.split_whitespace().filter(|w| !w.eq_ignore_ascii_case(&token)).collect::<Vec<_>>().join(" ")
                        } else {
                            format!("{} {token}", app.search.trim()).trim().to_string()
                        };
                    }
                }
            });
        }
        ui.separator();

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if !app.search.trim().is_empty() {
                let hits = app.store.search(&app.search);
                ui.label(RichText::new(format!("{} result(s)", hits.len())).weak());
                for (loc, meta) in hits {
                    let section = app.store.library.section(loc.section).map(|s| s.name.clone()).unwrap_or_default();
                    let resp = ui.selectable_label(Some(meta.id) == current, &meta.title);
                    let tags = meta.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ");
                    ui.label(RichText::new(format!("  {section}  {tags}")).small().weak());
                    if resp.clicked() {
                        action = Some(Action::Open(loc));
                    }
                }
                return;
            }
            for nb in &app.store.library.notebooks {
                let nb_resp = egui::CollapsingHeader::new(RichText::new(format!("📓 {}", nb.name)).strong())
                    .id_salt(nb.id)
                    .default_open(true)
                    .show(ui, |ui| {
                        for sec in &nb.sections {
                            let cloud = if sec.onenote.is_some() { " ☁" } else { "" };
                            let sec_resp = egui::CollapsingHeader::new(format!("📑 {}{cloud}", sec.name))
                                .id_salt(sec.id)
                                .default_open(sec.pages.iter().any(|p| Some(p.id) == current))
                                .show(ui, |ui| {
                                    for p in &sec.pages {
                                        let label = if p.title.is_empty() { "Untitled" } else { &p.title };
                                        let r = ui.selectable_label(Some(p.id) == current, label);
                                        let r = if p.tags.is_empty() {
                                            r
                                        } else {
                                            r.on_hover_text(p.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" "))
                                        };
                                        if r.clicked() {
                                            action = Some(Action::Open(PageLocation { notebook: nb.id, section: sec.id, page: p.id }));
                                        }
                                        r.context_menu(|ui| {
                                            if ui.button("Rename").clicked() {
                                                action = Some(Action::Rename(Target::Page(p.id), p.title.clone()));
                                            }
                                            if ui.button("Delete").clicked() {
                                                action = Some(Action::Delete(Target::Page(p.id), p.title.clone()));
                                            }
                                        });
                                    }
                                    if ui.small_button("+ Page").clicked() {
                                        action = Some(Action::NewPage(sec.id));
                                    }
                                });
                            sec_resp.header_response.context_menu(|ui| {
                                if ui.button("New page").clicked() {
                                    action = Some(Action::NewPage(sec.id));
                                }
                                if ui.button("Rename").clicked() {
                                    action = Some(Action::Rename(Target::Section(sec.id), sec.name.clone()));
                                }
                                if ui.button("Save section to OneNote").clicked() {
                                    action = Some(Action::Push(sec.id));
                                }
                                ui.separator();
                                if ui.button("Delete").clicked() {
                                    action = Some(Action::Delete(Target::Section(sec.id), sec.name.clone()));
                                }
                            });
                        }
                        if ui.small_button("+ Section").clicked() {
                            action = Some(Action::NewSection(nb.id));
                        }
                    });
                nb_resp.header_response.context_menu(|ui| {
                    if ui.button("New section").clicked() {
                        action = Some(Action::NewSection(nb.id));
                    }
                    if ui.button("Rename").clicked() {
                        action = Some(Action::Rename(Target::Notebook(nb.id), nb.name.clone()));
                    }
                    ui.separator();
                    if ui.button("Delete").clicked() {
                        action = Some(Action::Delete(Target::Notebook(nb.id), nb.name.clone()));
                    }
                });
            }
            ui.add_space(6.0);
            if ui.button("+ Notebook").clicked() {
                action = Some(Action::NewNotebook);
            }
            ui.label(RichText::new("Right-click items for more options").small().weak());
        });
    });

    match action {
        Some(Action::Open(loc)) => app.open_page(loc),
        Some(Action::NewPage(s)) => app.new_page(s),
        Some(Action::NewSection(nb)) => {
            if let Some(s) = app.new_section(nb) {
                app.new_page(s);
            }
        }
        Some(Action::NewNotebook) => app.new_notebook(),
        Some(Action::Rename(target, text)) => app.dialog = Some(Dialog::Rename { target, text }),
        Some(Action::Delete(target, label)) => app.dialog = Some(Dialog::ConfirmDelete { target, label }),
        Some(Action::Push(s)) => app.push_section_to_onenote(s),
        None => {}
    }
}

pub fn dialogs(app: &mut SketcherApp, ctx: &egui::Context) {
    let Some(dialog) = app.dialog.as_mut() else { return };
    let mut close = false;
    let mut commit = None;
    match dialog {
        Dialog::Rename { target, text } => {
            egui::Window::new("Rename").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                let r = ui.text_edit_singleline(text);
                r.request_focus();
                ui.horizontal(|ui| {
                    if ui.button("Rename").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit = Some((*target, text.clone(), false));
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                });
            });
        }
        Dialog::ConfirmDelete { target, label } => {
            egui::Window::new("Delete?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                let what = match target {
                    Target::Notebook(_) => "notebook and all its sections and pages",
                    Target::Section(_) => "section and all its pages",
                    Target::Page(_) => "page",
                };
                ui.label(format!("Delete the {what} \"{label}\"?"));
                ui.label(RichText::new("This only affects the local library, not OneNote.").small().weak());
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Delete").color(ui.visuals().error_fg_color)).clicked() {
                        commit = Some((*target, String::new(), true));
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                });
            });
        }
    }
    if let Some((target, text, delete)) = commit {
        if delete {
            app.delete(target);
        } else {
            app.rename(target, text);
        }
        close = true;
    }
    if close {
        app.dialog = None;
    }
}
