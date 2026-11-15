//! Sketcher core: the platform-independent data model.
//!
//! This crate has no knowledge of rendering or storage. Everything else
//! in the workspace depends on it, never the other way round.

pub mod geometry;
pub mod history;
pub mod model;
pub mod tags;

pub use geometry::{Point, Rect};
pub use history::History;
pub use model::*;
