//! The drawing surface: pen/eraser/pan input, zoom, and cached stroke meshes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::{Color32, Mesh, Pos2, Rect, Sense, Shape, Stroke as UiStroke, Vec2};
use sketch_core::geometry::InputSmoother;
use sketch_core::history::Edit;
use sketch_core::{Color, Id, Page, Pen, Point, Stroke};
use sketch_render::paper::{self, Mark, PAPER_COLOR};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Eraser,
    Pan,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Source {
    Mouse,
    Touch(u64),
}

struct Active {
    stroke: Stroke,
    source: Source,
}

pub struct Canvas {
    pub zoom: f32,
    /// Screen offset of the page origin relative to the canvas' top-left corner.
    pub offset: Vec2,
    pub eraser_radius: f32,
    pub smoothing: f32,
    page_meshes: HashMap<Id, Arc<Mesh>>,
    screen: Vec<Shape>,
    screen_key: Option<(f32, Vec2, Pos2, Vec2, u64)>,
    revision: u64,
    active: Option<Active>,
    smoother: InputSmoother,
    /// Strokes erased in the current eraser drag (hidden until committed).
    erasing: Vec<(usize, Stroke)>,
    erasing_ids: HashSet<Id>,
    eraser_down: bool,
    panning: bool,
    fit_pending: bool,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            offset: Vec2::new(24.0, 24.0),
            eraser_radius: 10.0,
            smoothing: 0.35,
            page_meshes: HashMap::new(),
            screen: Vec::new(),
            screen_key: None,
            revision: 0,
            active: None,
            smoother: InputSmoother::new(0.35, 0.4),
            erasing: Vec::new(),
            erasing_ids: HashSet::new(),
            eraser_down: false,
            panning: false,
            fit_pending: true,
        }
    }
}

fn color32(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn page_mesh(stroke: &Stroke) -> Mesh {
    let mut m = sketch_render::Mesh::default();
    sketch_render::tessellate_stroke(stroke, &mut m);
    let color = color32(stroke.pen.effective_color());
    let mut mesh = Mesh::default();
    mesh.vertices.reserve(m.positions.len());
    for [x, y] in m.positions {
        mesh.colored_vertex(Pos2::new(x, y), color);
    }
    mesh.indices = m.indices;
    mesh
}

impl Canvas {
    /// Call whenever the page's strokes change outside the canvas (undo, load, …).
    pub fn invalidate(&mut self) {
        self.revision += 1;
    }

    /// Drops all cached geometry (when switching pages).
    pub fn reset_for_new_page(&mut self) {
        self.page_meshes.clear();
        self.active = None;
        self.erasing.clear();
        self.erasing_ids.clear();
        self.fit_pending = true;
        self.invalidate();
    }

    pub fn request_fit(&mut self) {
        self.fit_pending = true;
    }

    pub fn is_drawing(&self) -> bool {
        self.active.is_some() || self.eraser_down
    }

    pub fn zoom_by(&mut self, factor: f32, anchor: Vec2) {
        let new = (self.zoom * factor).clamp(0.1, 8.0);
        let f = new / self.zoom;
        self.offset = anchor - (anchor - self.offset) * f;
        self.zoom = new;
    }

    fn to_page(&self, origin: Pos2, p: Pos2) -> (f32, f32) {
        let v = (p - origin - self.offset) / self.zoom;
        (v.x, v.y)
    }

    fn to_screen(&self, origin: Pos2, x: f32, y: f32) -> Pos2 {
        origin + self.offset + Vec2::new(x, y) * self.zoom
    }

    /// Draws the page and handles input. Returns a finished edit to record in history.
    pub fn show(&mut self, ui: &mut egui::Ui, page: &Page, pen: &Pen, tool: Tool) -> Option<Edit> {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let origin = rect.min;

        if self.fit_pending && rect.width() > 50.0 {
            self.fit_pending = false;
            let z = ((rect.width() - 48.0) / page.width).clamp(0.1, 2.0);
            self.zoom = z;
            self.offset = Vec2::new(((rect.width() - page.width * z) * 0.5).max(24.0), 24.0);
            self.invalidate();
        }

        let mut edit = None;
        self.handle_view_input(ui, &response, rect, tool);
        if tool == Tool::Pen && !self.panning {
            edit = self.handle_pen_input(ui, rect, pen);
        } else if tool == Tool::Eraser && !self.panning {
            edit = self.handle_eraser_input(ui, &response, rect, page);
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        self.paint_paper(&painter, origin, page);
        self.paint_strokes(&painter, rect, page);

        if let Some(active) = &self.active {
            let mut m = page_mesh(&active.stroke);
            for v in &mut m.vertices {
                v.pos = self.to_screen(origin, v.pos.x, v.pos.y);
            }
            painter.add(Shape::mesh(m));
            ui.ctx().request_repaint();
        }

        if let Some(pos) = response.hover_pos() {
            match tool {
                Tool::Eraser => {
                    painter.circle_stroke(pos, self.eraser_radius * self.zoom, UiStroke::new(1.0, Color32::GRAY));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                }
                Tool::Pen => ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair),
                Tool::Pan => ui.ctx().set_cursor_icon(if self.panning {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                }),
            }
        }
        edit
    }

    fn handle_view_input(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect, tool: Tool) {
        let (zoom_delta, scroll, space, middle, hover) = ui.input(|i| {
            (
                i.zoom_delta(),
                i.smooth_scroll_delta(),
                i.key_down(egui::Key::Space),
                i.pointer.middle_down(),
                i.pointer.hover_pos(),
            )
        });
        if let Some(hover) = hover.filter(|p| rect.contains(*p)) {
            if zoom_delta != 1.0 {
                self.zoom_by(zoom_delta, hover - rect.min);
            } else if scroll != Vec2::ZERO {
                self.offset += scroll;
            }
        }
        let pan_drag = (tool == Tool::Pan || space || middle) && response.dragged();
        if pan_drag {
            self.offset += response.drag_delta();
        }
        self.panning = pan_drag || (space && tool != Tool::Pan);
    }

    fn begin(&mut self, pen: &Pen, source: Source, p: Point) {
        self.smoother = InputSmoother::new(self.smoothing, 0.4 / self.zoom);
        let mut stroke = Stroke::new(*pen);
        if let Some(p) = self.smoother.push(p) {
            stroke.points.push(p);
        }
        self.active = Some(Active { stroke, source });
    }

    fn extend(&mut self, p: Point) {
        if let (Some(a), Some(p)) = (self.active.as_mut(), self.smoother.push(p)) {
            a.stroke.points.push(p);
        }
    }

    fn finish(&mut self) -> Option<Edit> {
        let a = self.active.take()?;
        (!a.stroke.points.is_empty()).then_some(Edit::AddStroke(a.stroke))
    }

    /// Processes raw events in order for full input fidelity (not just one
    /// sample per frame). Pen/touch events carry pressure; the mouse
    /// emulation egui generates for them is ignored while a touch stroke is active.
    fn handle_pen_input(&mut self, ui: &egui::Ui, rect: Rect, pen: &Pen) -> Option<Edit> {
        let events = ui.input(|i| i.events.clone());
        let origin = rect.min;
        let mut done = None;
        for ev in events {
            match ev {
                egui::Event::Touch { id, phase, pos, force, .. } => {
                    let (x, y) = self.to_page(origin, pos);
                    let pt = Point::new(x, y, force.unwrap_or(1.0).clamp(0.0, 1.0));
                    let src = Source::Touch(id.0);
                    match phase {
                        egui::TouchPhase::Start if rect.contains(pos) => {
                            // A touch supersedes a mouse-emulated stroke that started this frame.
                            if self.active.as_ref().is_none_or(|a| a.source == Source::Mouse) {
                                self.begin(pen, src, pt);
                            }
                        }
                        egui::TouchPhase::Move if self.active.as_ref().is_some_and(|a| a.source == src) => {
                            self.extend(pt)
                        }
                        egui::TouchPhase::End if self.active.as_ref().is_some_and(|a| a.source == src) => {
                            self.extend(pt);
                            done = self.finish().or(done);
                        }
                        egui::TouchPhase::Cancel if self.active.as_ref().is_some_and(|a| a.source == src) => {
                            done = self.finish().or(done);
                        }
                        _ => {}
                    }
                }
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, .. } => {
                    let (x, y) = self.to_page(origin, pos);
                    let pt = Point::new(x, y, 0.6);
                    if pressed && self.active.is_none() && rect.contains(pos) {
                        self.begin(pen, Source::Mouse, pt);
                    } else if !pressed && self.active.as_ref().is_some_and(|a| a.source == Source::Mouse) {
                        self.extend(pt);
                        done = self.finish().or(done);
                    }
                }
                egui::Event::PointerMoved(pos) if self.active.as_ref().is_some_and(|a| a.source == Source::Mouse) => {
                    let (x, y) = self.to_page(origin, pos);
                    self.extend(Point::new(x, y, 0.6));
                }
                egui::Event::PointerGone if self.active.as_ref().is_some_and(|a| a.source == Source::Mouse) => {
                    done = self.finish().or(done);
                }
                _ => {}
            }
        }
        done
    }

    fn handle_eraser_input(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect, page: &Page) -> Option<Edit> {
        let down = response.is_pointer_button_down_on() || response.dragged();
        if down {
            self.eraser_down = true;
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                let (x, y) = self.to_page(rect.min, pos);
                for i in page.strokes_hit(x, y, self.eraser_radius) {
                    let s = &page.strokes[i];
                    if self.erasing_ids.insert(s.id) {
                        self.erasing.push((i, s.clone()));
                        self.invalidate();
                    }
                }
            }
            None
        } else if self.eraser_down {
            self.eraser_down = false;
            self.erasing_ids.clear();
            let mut removed = std::mem::take(&mut self.erasing);
            removed.sort_by_key(|(i, _)| *i);
            self.invalidate();
            (!removed.is_empty()).then_some(Edit::RemoveStrokes(removed))
        } else {
            None
        }
    }

    fn paint_paper(&self, painter: &egui::Painter, origin: Pos2, page: &Page) {
        let page_rect = Rect::from_min_max(self.to_screen(origin, 0.0, 0.0), self.to_screen(origin, page.width, page.height));
        painter.add(egui::epaint::Shadow { offset: [0, 2], blur: 12, spread: 0, color: Color32::from_black_alpha(60) }.as_shape(page_rect, 2.0));
        painter.rect_filled(page_rect, 2.0, color32(PAPER_COLOR));
        let clip = painter.clip_rect();
        for mark in paper::marks(page.paper, page.width, page.height) {
            match mark {
                Mark::Line { from, to, width, color } => {
                    let a = self.to_screen(origin, from[0], from[1]);
                    let b = self.to_screen(origin, to[0], to[1]);
                    if Rect::from_two_pos(a, b).expand(1.0).intersects(clip) {
                        painter.line_segment([a, b], UiStroke::new((width * self.zoom).max(0.5), color32(color)));
                    }
                }
                Mark::Dot { center, radius, color } => {
                    let c = self.to_screen(origin, center[0], center[1]);
                    if clip.contains(c) {
                        painter.circle_filled(c, (radius * self.zoom).max(0.6), color32(color));
                    }
                }
            }
        }
    }

    /// Strokes are tessellated once (page space) and re-projected to screen
    /// only when the view or the page changes; otherwise last frame's shapes are reused.
    fn paint_strokes(&mut self, painter: &egui::Painter, rect: Rect, page: &Page) {
        let key = (self.zoom, self.offset, rect.min, rect.size(), self.revision);
        if self.screen_key != Some(key) {
            self.screen_key = Some(key);
            let live: HashSet<Id> = page.strokes.iter().map(|s| s.id).collect();
            self.page_meshes.retain(|id, _| live.contains(id));
            let (x0, y0) = self.to_page(rect.min, rect.min);
            let (x1, y1) = self.to_page(rect.min, rect.max);
            let visible = sketch_core::Rect { min_x: x0, min_y: y0, max_x: x1, max_y: y1 };
            self.screen.clear();
            for stroke in sketch_render::paint_order(&page.strokes) {
                if self.erasing_ids.contains(&stroke.id) || !stroke.bounds().intersects(&visible) {
                    continue;
                }
                let base = self.page_meshes.entry(stroke.id).or_insert_with(|| Arc::new(page_mesh(stroke)));
                let mut m = (**base).clone();
                for v in &mut m.vertices {
                    v.pos = rect.min + self.offset + v.pos.to_vec2() * self.zoom;
                }
                self.screen.push(Shape::mesh(m));
            }
        }
        painter.extend(self.screen.iter().cloned());
    }
}
