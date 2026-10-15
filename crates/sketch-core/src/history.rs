use crate::model::{Page, Stroke};

/// A reversible edit to a page's ink.
#[derive(Debug, Clone)]
pub enum Edit {
    AddStroke(Stroke),
    /// Strokes removed, with their original indices (ascending).
    RemoveStrokes(Vec<(usize, Stroke)>),
    /// Strokes replaced wholesale, e.g. clear page.
    Replace { before: Vec<Stroke>, after: Vec<Stroke> },
}

/// Undo/redo stack for one page.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

const MAX_HISTORY: usize = 500;

impl History {
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Applies `edit` to `page` and records it.
    pub fn apply(&mut self, page: &mut Page, edit: Edit) {
        apply(page, &edit);
        self.undo.push(edit);
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        page.touch();
    }

    pub fn undo(&mut self, page: &mut Page) -> bool {
        let Some(edit) = self.undo.pop() else { return false };
        revert(page, &edit);
        self.redo.push(edit);
        page.touch();
        true
    }

    pub fn redo(&mut self, page: &mut Page) -> bool {
        let Some(edit) = self.redo.pop() else { return false };
        apply(page, &edit);
        self.undo.push(edit);
        page.touch();
        true
    }
}

fn apply(page: &mut Page, edit: &Edit) {
    match edit {
        Edit::AddStroke(s) => page.strokes.push(s.clone()),
        Edit::RemoveStrokes(removed) => {
            for (i, _) in removed.iter().rev() {
                page.strokes.remove(*i);
            }
        }
        Edit::Replace { after, .. } => page.strokes = after.clone(),
    }
}

fn revert(page: &mut Page, edit: &Edit) {
    match edit {
        Edit::AddStroke(s) => {
            if let Some(i) = page.strokes.iter().rposition(|x| x.id == s.id) {
                page.strokes.remove(i);
            }
        }
        Edit::RemoveStrokes(removed) => {
            for (i, s) in removed {
                page.strokes.insert(*i, s.clone());
            }
        }
        Edit::Replace { before, .. } => page.strokes = before.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Pen;

    #[test]
    fn undo_redo_remove() {
        let mut page = Page::new("t");
        let mut h = History::default();
        let strokes: Vec<Stroke> = (0..4).map(|_| Stroke::new(Pen::default())).collect();
        for s in &strokes {
            h.apply(&mut page, Edit::AddStroke(s.clone()));
        }
        let removed = vec![(1, strokes[1].clone()), (3, strokes[3].clone())];
        h.apply(&mut page, Edit::RemoveStrokes(removed));
        assert_eq!(page.strokes.len(), 2);
        h.undo(&mut page);
        assert_eq!(page.strokes, strokes);
        h.redo(&mut page);
        assert_eq!(page.strokes, vec![strokes[0].clone(), strokes[2].clone()]);
    }
}
