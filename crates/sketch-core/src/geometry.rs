use serde::{Deserialize, Serialize};

/// A sampled pen position in page units (1 unit = 1 CSS pixel at 96 DPI).
/// Serialized compactly as `[x, y, pressure]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(from = "[f32; 3]", into = "[f32; 3]")]
pub struct Point {
    pub x: f32,
    pub y: f32,
    /// Normalized pressure, 0.0..=1.0.
    pub pressure: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32, pressure: f32) -> Self {
        Self { x, y, pressure }
    }

    pub fn distance(&self, other: &Point) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

impl From<[f32; 3]> for Point {
    fn from([x, y, pressure]: [f32; 3]) -> Self {
        Self { x, y, pressure }
    }
}

impl From<Point> for [f32; 3] {
    fn from(p: Point) -> Self {
        [p.x, p.y, p.pressure]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl Rect {
    pub const EMPTY: Rect = Rect {
        min_x: f32::INFINITY,
        min_y: f32::INFINITY,
        max_x: f32::NEG_INFINITY,
        max_y: f32::NEG_INFINITY,
    };

    pub fn is_empty(&self) -> bool {
        self.min_x > self.max_x || self.min_y > self.max_y
    }

    pub fn include(&mut self, x: f32, y: f32) {
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }

    pub fn expand(&self, by: f32) -> Rect {
        Rect {
            min_x: self.min_x - by,
            min_y: self.min_y - by,
            max_x: self.max_x + by,
            max_y: self.max_y + by,
        }
    }

    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_y <= other.max_y
            && self.max_y >= other.min_y
    }

    pub fn width(&self) -> f32 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f32 {
        self.max_y - self.min_y
    }
}

/// Squared distance from `p` to segment `a`-`b`.
pub fn dist_sq_point_segment(px: f32, py: f32, a: &Point, b: &Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len_sq = dx * dx + dy * dy;
    let t = if len_sq <= f32::EPSILON {
        0.0
    } else {
        (((px - a.x) * dx + (py - a.y) * dy) / len_sq).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.x + t * dx, a.y + t * dy);
    (px - cx).powi(2) + (py - cy).powi(2)
}

/// Smooths raw input samples as they arrive: exponential smoothing plus a
/// minimum-distance filter. Keeps latency low (no look-ahead).
#[derive(Debug, Clone)]
pub struct InputSmoother {
    /// 0 = no smoothing, values near 1 = heavy smoothing.
    pub strength: f32,
    pub min_distance: f32,
    last: Option<Point>,
}

impl InputSmoother {
    pub fn new(strength: f32, min_distance: f32) -> Self {
        Self { strength: strength.clamp(0.0, 0.95), min_distance, last: None }
    }

    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Returns the point to append, or `None` if the sample was filtered out.
    pub fn push(&mut self, raw: Point) -> Option<Point> {
        let Some(last) = self.last else {
            self.last = Some(raw);
            return Some(raw);
        };
        let a = 1.0 - self.strength;
        let p = Point {
            x: last.x + (raw.x - last.x) * a,
            y: last.y + (raw.y - last.y) * a,
            pressure: last.pressure + (raw.pressure - last.pressure) * a,
        };
        if p.distance(&last) < self.min_distance {
            return None;
        }
        self.last = Some(p);
        Some(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_serializes_as_array() {
        let s = serde_json::to_string(&Point::new(1.0, 2.0, 0.5)).unwrap();
        assert_eq!(s, "[1.0,2.0,0.5]");
        let p: Point = serde_json::from_str(&s).unwrap();
        assert_eq!(p, Point::new(1.0, 2.0, 0.5));
    }

    #[test]
    fn segment_distance() {
        let a = Point::new(0.0, 0.0, 1.0);
        let b = Point::new(10.0, 0.0, 1.0);
        assert_eq!(dist_sq_point_segment(5.0, 3.0, &a, &b), 9.0);
        assert_eq!(dist_sq_point_segment(-4.0, 3.0, &a, &b), 25.0);
    }
}
