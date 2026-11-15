use coolmus_core::{Color, Paper};

pub const RULE_SPACING: f32 = 32.0;
pub const GRID_SPACING: f32 = 24.0;
pub const PAPER_COLOR: Color = Color::rgb(0xfd, 0xfd, 0xfb);
pub const RULE_COLOR: Color = Color::rgb(0xc9, 0xd8, 0xea);
pub const MARGIN_COLOR: Color = Color::rgb(0xf0, 0xb4, 0xb4);

/// One element of a paper's background pattern, in page coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    Line { from: [f32; 2], to: [f32; 2], width: f32, color: Color },
    Dot { center: [f32; 2], radius: f32, color: Color },
}

pub fn marks(paper: Paper, width: f32, height: f32) -> Vec<Mark> {
    let mut out = Vec::new();
    let hline = |y: f32, color| Mark::Line { from: [0.0, y], to: [width, y], width: 1.0, color };
    let vline = |x: f32, color| Mark::Line { from: [x, 0.0], to: [x, height], width: 1.0, color };
    match paper {
        Paper::Blank => {}
        Paper::Lined => {
            let mut y = RULE_SPACING * 3.0;
            while y < height {
                out.push(hline(y, RULE_COLOR));
                y += RULE_SPACING;
            }
            out.push(vline(80.0, MARGIN_COLOR));
        }
        Paper::Grid => {
            let mut y = GRID_SPACING;
            while y < height {
                out.push(hline(y, RULE_COLOR));
                y += GRID_SPACING;
            }
            let mut x = GRID_SPACING;
            while x < width {
                out.push(vline(x, RULE_COLOR));
                x += GRID_SPACING;
            }
        }
        Paper::Dotted => {
            let mut y = GRID_SPACING;
            while y < height {
                let mut x = GRID_SPACING;
                while x < width {
                    out.push(Mark::Dot { center: [x, y], radius: 1.2, color: RULE_COLOR });
                    x += GRID_SPACING;
                }
                y += GRID_SPACING;
            }
        }
    }
    out
}
