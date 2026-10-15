//! Top toolbar (tools, pen settings, page title & tags), status bar, settings.

use egui::{Color32, RichText};
use onenote::ExportMode;
use sketch_core::{tags, Color, Paper, PenStyle};

use crate::app::SketcherApp;
use crate::canvas::Tool;

const PALETTE: [Color; 10] = [
    Color::rgb(0x1a, 0x1a, 0x1a),
    Color::rgb(0x5f, 0x63, 0x68),
    Color::rgb(0xd9, 0x30, 0x25),
    Color::rgb(0xf2, 0x8b, 0x00),
    Color::rgb(0xf9, 0xd4, 0x00),
    Color::rgb(0x1e, 0x8e, 0x3e),
    Color::rgb(0x00, 0x96, 0x88),
    Color::rgb(0x1a, 0x73, 0xe8),
    Color::rgb(0x7b, 0x1f, 0xa2),
    Color::rgb(0xe9, 0x1e, 0x63),
];

fn c32(c: Color) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn swatch(ui: &mut egui::Ui, color: Color, selected: bool) -> egui::Response {
    let size = egui::vec2(20.0, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter();
    p.circle_filled(rect.center(), 8.0, c32(color));
    if selected {
        p.circle_stroke(rect.center(), 10.0, egui::Stroke::new(2.0, ui.visuals().selection.bg_fill));
    } else if resp.hovered() {
        p.circle_stroke(rect.center(), 10.0, egui::Stroke::new(1.0, ui.visuals().weak_text_color()));
    }
    resp
}

pub fn show(app: &mut SketcherApp, ui: &mut egui::Ui, now: f64) {
    egui::Panel::top("toolbar").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for (tool, label, tip) in [
                (Tool::Pen, "✏ Pen", "Draw (P)"),
                (Tool::Eraser, "🗑 Eraser", "Erase whole strokes (E)"),
                (Tool::Pan, "✋ Pan", "Move the page (H) — or hold Space / middle mouse"),
            ] {
                ui.selectable_value(&mut app.tool, tool, label).on_hover_text(tip);
            }
            ui.separator();

            let pen = &mut app.settings.pen;
            for c in PALETTE {
                if swatch(ui, c, pen.color.with_alpha(255) == c).clicked() {
                    pen.color = c;
                    app.tool = Tool::Pen;
                }
            }
            let mut c = Color32::from_rgb(pen.color.r, pen.color.g, pen.color.b);
            if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed() {
                pen.color = Color::rgb(c.r(), c.g(), c.b());
            }
            ui.separator();

            ui.label("Width");
            ui.add(egui::Slider::new(&mut pen.width, 0.5..=40.0).logarithmic(true).max_decimals(1));
            egui::ComboBox::from_id_salt("pen-style").selected_text(pen.style.label()).show_ui(ui, |ui| {
                for s in PenStyle::ALL {
                    ui.selectable_value(&mut pen.style, s, s.label());
                }
            });
            ui.toggle_value(&mut pen.pressure_sensitive, "Pressure").on_hover_text("Pen pressure changes stroke width");
            if app.tool == Tool::Eraser {
                ui.label("Eraser");
                ui.add(egui::Slider::new(&mut app.canvas.eraser_radius, 2.0..=60.0).max_decimals(0));
            }
            ui.separator();

            let (can_undo, can_redo) = app.current.as_ref().map_or((false, false), |c| (c.history.can_undo(), c.history.can_redo()));
            if ui.add_enabled(can_undo, egui::Button::new("⟲")).on_hover_text("Undo (Ctrl+Z)").clicked() {
                app.undo(now);
            }
            if ui.add_enabled(can_redo, egui::Button::new("⟳")).on_hover_text("Redo (Ctrl+Y)").clicked() {
                app.redo(now);
            }
            if ui.button("−").on_hover_text("Zoom out").clicked() {
                app.canvas.zoom_by(1.0 / 1.2, egui::vec2(300.0, 300.0));
            }
            if ui.button(format!("{:.0}%", app.canvas.zoom * 100.0)).on_hover_text("Fit width (Ctrl+0)").clicked() {
                app.canvas.request_fit();
            }
            if ui.button("+").on_hover_text("Zoom in").clicked() {
                app.canvas.zoom_by(1.2, egui::vec2(300.0, 300.0));
            }
            ui.separator();

            ui.menu_button("Page ⏷", |ui| {
                if let Some(cur) = app.current.as_mut() {
                    let mut paper = cur.page.paper;
                    ui.label("Paper");
                    for p in Paper::ALL {
                        ui.radio_value(&mut paper, p, p.label());
                    }
                    if paper != cur.page.paper {
                        cur.page.paper = paper;
                        app.mark_dirty(now);
                    }
                    ui.separator();
                    if ui.button("Extend page ↓").clicked() {
                        if let Some(cur) = app.current.as_mut() {
                            cur.page.height += sketch_core::DEFAULT_PAGE_SIZE.1 / 2.0;
                        }
                        app.mark_dirty(now);
                    }
                    if ui.button("Clear page").clicked() {
                        app.clear_page(now);
                        ui.close();
                    }
                    if ui.button("Export PNG…").clicked() {
                        ui.close();
                        app.export_png();
                    }
                }
            });
            if ui.button("☁ OneNote").clicked() {
                app.onenote.open = !app.onenote.open;
            }
            if ui.button("⚙").on_hover_text("Settings").clicked() {
                app.show_settings = !app.show_settings;
            }
        });

        // Page header: title and tags.
        let mut changed = false;
        let all_tags = app.store.library.all_tags();
        if let Some(cur) = app.current.as_mut() {
            ui.horizontal_wrapped(|ui| {
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut cur.page.title).desired_width(260.0).font(egui::TextStyle::Heading).hint_text("Title"))
                    .changed();
                ui.separator();
                let mut remove = None;
                for (i, t) in cur.page.tags.iter().enumerate() {
                    if ui.button(RichText::new(format!("#{t} ×")).small()).on_hover_text("Remove tag").clicked() {
                        remove = Some(i);
                    }
                }
                if let Some(i) = remove {
                    cur.page.tags.remove(i);
                    changed = true;
                }
                let resp = ui.add(egui::TextEdit::singleline(&mut app.tag_input).desired_width(120.0).hint_text("+ add tag"));
                let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if submit {
                    for word in app.tag_input.split([',', ' ']) {
                        changed |= tags::add_tag(&mut cur.page.tags, word);
                    }
                    app.tag_input.clear();
                    resp.request_focus();
                }
                // Suggest existing tags while typing.
                let typed = app.tag_input.trim().trim_start_matches('#').to_lowercase();
                if !typed.is_empty() {
                    let suggestions: Vec<&String> = all_tags
                        .iter()
                        .filter(|t| t.to_lowercase().starts_with(&typed) && !cur.page.tags.contains(t))
                        .take(5)
                        .collect();
                    for t in suggestions {
                        if ui.small_button(format!("#{t}")).clicked() {
                            changed |= tags::add_tag(&mut cur.page.tags, t);
                            app.tag_input.clear();
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let busy = app.sync.busy;
                    if ui.add_enabled(!busy, egui::Button::new("Save to OneNote")).clicked() {
                        app.pending_onenote_save = true;
                    }
                    if busy {
                        ui.spinner();
                    }
                    if let Some(link) = &cur.page.onenote {
                        ui.label(RichText::new("☁ linked").small().weak()).on_hover_text(format!("OneNote page {}", link.page_id));
                    }
                });
            });
        }
        if changed {
            app.mark_dirty(now);
        }
        if std::mem::take(&mut app.pending_onenote_save) {
            app.save_current_to_onenote();
        }
        ui.add_space(2.0);
    });
}

pub fn status_bar(app: &mut SketcherApp, ui: &mut egui::Ui) {
    egui::Panel::bottom("status").show(ui, |ui| {
        ui.horizontal(|ui| {
            if app.sync.busy {
                ui.spinner();
            }
            ui.label(RichText::new(&app.status).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(cur) = &app.current {
                    let n = cur.page.strokes.len();
                    let state = if cur.dirty { "unsaved" } else { "saved" };
                    ui.label(RichText::new(format!("{n} strokes · {state}")).small().weak());
                }
            });
        });
    });
}

pub fn settings_window(app: &mut SketcherApp, ctx: &egui::Context) {
    let mut open = app.show_settings;
    egui::Window::new("Settings").open(&mut open).resizable(false).show(ctx, |ui| {
        ui.heading("Drawing");
        if ui.add(egui::Slider::new(&mut app.settings.smoothing, 0.0..=0.9).text("Stroke smoothing")).changed() {
            app.canvas.smoothing = app.settings.smoothing;
        }
        ui.separator();
        ui.heading("OneNote");
        ui.label("Save pages as:");
        ui.radio_value(&mut app.settings.export_mode, ExportMode::NativeInk, "Native OneNote ink (editable in OneNote)");
        ui.radio_value(&mut app.settings.export_mode, ExportMode::Image, "Picture (updated in place, keeps other page content)");
        ui.checkbox(&mut app.settings.replace_old, "When re-saving, replace the previous OneNote page");
        ui.label(
            RichText::new("OneNote's API cannot modify existing ink, so re-saving an ink page creates a new page.")
                .small()
                .weak(),
        );
        ui.separator();
        ui.label(RichText::new(format!("Library: {}", app.store.root().display())).small().weak());
        if ui.button("Save settings").clicked() {
            app.settings.save();
            app.status = "Settings saved".into();
        }
    });
    app.show_settings = open;
}

pub fn error_toasts(app: &mut SketcherApp, ctx: &egui::Context) {
    if app.errors.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("errors"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -36.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(420.0);
                let mut dismiss = None;
                for (i, e) in app.errors.iter().enumerate().rev().take(4) {
                    ui.horizontal(|ui| {
                        ui.colored_label(ui.visuals().error_fg_color, "⚠");
                        ui.label(e);
                        if ui.small_button("×").clicked() {
                            dismiss = Some(i);
                        }
                    });
                }
                if let Some(i) = dismiss {
                    app.errors.remove(i);
                }
                if app.errors.len() > 1 && ui.small_button("Dismiss all").clicked() {
                    app.errors.clear();
                }
            });
        });
}
