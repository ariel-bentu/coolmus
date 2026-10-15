//! OneNote panel: configure, sign in, browse notebooks/sections/pages, import.

use egui::RichText;

use crate::app::SketcherApp;
use crate::sync::Job;

pub fn show(app: &mut SketcherApp, ctx: &egui::Context) {
    let mut open = app.onenote.open;
    egui::Window::new("☁ OneNote")
        .open(&mut open)
        .default_size([420.0, 520.0])
        .resizable(true)
        .show(ctx, |ui| body(app, ui));
    app.onenote.open = open;
}

fn body(app: &mut SketcherApp, ui: &mut egui::Ui) {
    let busy = app.sync.busy;

    // 1. Configuration.
    if app.settings.auth.client_id.trim().is_empty() || !app.sync.signed_in && app.onenote.device_code.is_none() {
        egui::CollapsingHeader::new("Connection").default_open(app.settings.auth.client_id.is_empty()).show(ui, |ui| {
            ui.label(RichText::new("Sketcher talks to OneNote through Microsoft Graph. Register an app in Microsoft Entra ID (see README) and paste its Application (client) ID.").small());
            egui::Grid::new("auth").num_columns(2).show(ui, |ui| {
                ui.label("Client ID");
                ui.text_edit_singleline(&mut app.onenote.client_id_edit);
                ui.end_row();
                ui.label("Tenant");
                ui.text_edit_singleline(&mut app.onenote.tenant_edit).on_hover_text("common, consumers, organizations or your tenant ID");
                ui.end_row();
            });
            if ui.button("Apply").clicked() {
                app.settings.auth.client_id = app.onenote.client_id_edit.trim().to_string();
                app.settings.auth.tenant = match app.onenote.tenant_edit.trim() {
                    "" => "common".to_string(),
                    t => t.to_string(),
                };
                app.settings.save();
                app.sync.send(Job::Configure(app.settings.auth.clone()));
            }
        });
    }

    // 2. Sign-in.
    if let Some(dc) = app.onenote.device_code.clone() {
        ui.separator();
        ui.heading("Sign in to Microsoft");
        ui.label(format!("1. Open {}", dc.verification_uri));
        ui.horizontal(|ui| {
            ui.label("2. Enter the code");
            ui.label(RichText::new(&dc.user_code).monospace().size(20.0).strong());
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(dc.user_code.clone());
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Open browser").clicked() {
                let _ = open::that(&dc.verification_uri);
            }
            if ui.button("Cancel").clicked() {
                app.sync.cancel();
                app.onenote.device_code = None;
            }
            ui.spinner();
        });
        return;
    }
    if !app.sync.signed_in {
        ui.separator();
        let ready = !app.settings.auth.client_id.trim().is_empty();
        if ui.add_enabled(ready && !busy, egui::Button::new("Sign in to OneNote")).clicked() {
            app.sync.send(Job::SignIn);
        }
        return;
    }

    // 3. Browse.
    ui.horizontal(|ui| {
        if ui.add_enabled(!busy, egui::Button::new("⟳ Refresh")).clicked() {
            app.sync.send(Job::ListNotebooks);
        }
        if ui.button("Sign out").clicked() {
            app.sync.send(Job::SignOut);
        }
        if busy {
            ui.spinner();
        }
    });
    if app.onenote.notebooks.is_empty() && !busy {
        ui.label("No notebooks loaded yet — press Refresh.");
    }

    let mut list_pages = None;
    egui::ScrollArea::vertical().id_salt("nbs").max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
        for nb in &app.onenote.notebooks {
            let selected_nb = app.onenote.selected_notebook.as_deref() == Some(&nb.id);
            let header = egui::CollapsingHeader::new(RichText::new(format!("📓 {}", nb.name)).strong().color(if selected_nb {
                ui.visuals().selection.stroke.color
            } else {
                ui.visuals().text_color()
            }))
            .id_salt(&nb.id)
            .show(ui, |ui| {
                for s in &nb.sections {
                    let sel = app.onenote.selected_section.as_ref().is_some_and(|(id, _, _)| *id == s.id);
                    if ui.selectable_label(sel, format!("📑 {}", s.name)).clicked() {
                        list_pages = Some((s.id.clone(), s.name.clone(), nb.id.clone()));
                    }
                }
            });
            if header.header_response.clicked() {
                app.onenote.selected_notebook = Some(nb.id.clone());
            }
        }
    });
    if let Some((id, name, nb)) = list_pages {
        app.onenote.selected_notebook = Some(nb.clone());
        app.onenote.selected_section = Some((id.clone(), name, nb));
        app.onenote.pages.clear();
        app.sync.send(Job::ListPages { section_id: id });
    }

    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.onenote.new_notebook_name).hint_text("New notebook name").desired_width(180.0));
        let name = app.onenote.new_notebook_name.trim().to_string();
        if ui.add_enabled(!name.is_empty() && !busy, egui::Button::new("Create notebook")).clicked() {
            app.sync.send(Job::CreateNotebook { name });
            app.onenote.new_notebook_name.clear();
        }
    });

    // 4. Pages of the selected section.
    ui.separator();
    let Some((sec_id, sec_name, nb_id)) = app.onenote.selected_section.clone() else {
        ui.label(RichText::new("Select a section to see its pages. The selected notebook/section is also where \"Save to OneNote\" puts new pages.").small().weak());
        return;
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("📑 {sec_name}")).strong());
        if ui.add_enabled(!busy, egui::Button::new("Import whole section")).clicked() {
            let local = app.ensure_local_section(&nb_id, &sec_id, &sec_name);
            app.sync.send(Job::ImportSection { remote_section: sec_id.clone(), local_section: local });
        }
    });
    let mut import = None;
    egui::ScrollArea::vertical().id_salt("pages").auto_shrink([false, true]).show(ui, |ui| {
        if app.onenote.pages.is_empty() {
            ui.label(RichText::new(if busy { "Loading…" } else { "No pages" }).weak());
        }
        for p in &app.onenote.pages {
            ui.horizontal(|ui| {
                let local = app.store.library.find_by_onenote_page(&p.id).is_some();
                if ui.add_enabled(!busy, egui::Button::new(if local { "Re-open" } else { "Open" })).clicked() {
                    import = Some(p.clone());
                }
                ui.label(if p.title.is_empty() { "Untitled" } else { &p.title });
                if let Some(m) = &p.last_modified {
                    ui.label(RichText::new(m.get(..10).unwrap_or(m)).small().weak());
                }
            });
        }
    });
    if let Some(page) = import {
        let local = app.ensure_local_section(&nb_id, &sec_id, &sec_name);
        app.onenote.pending_open = Some(page.id.clone());
        app.sync.send(Job::ImportPage { remote_section: sec_id, page, local_section: local });
    }
}
