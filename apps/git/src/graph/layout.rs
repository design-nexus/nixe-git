//! Lane positions never shift while paging. Holes are reused, never compacted.
use super::model::{Commit, Edge, Row};
#[derive(Clone)]
struct Lane {
    id: String,
    color: usize,
}
pub struct Layout {
    lanes: Vec<Option<Lane>>,
    next_color: usize,
}
impl Default for Layout {
    fn default() -> Self {
        Self { lanes: Vec::new(), next_color: 1 }
    }
}
impl Layout {
    fn free(&self) -> usize {
        self.lanes.iter().position(Option::is_none).unwrap_or(self.lanes.len())
    }
    fn insert(&mut self, lane: usize, id: String, color: usize) {
        if lane == self.lanes.len() {
            self.lanes.push(None);
        }
        self.lanes[lane] = Some(Lane { id, color });
    }
    fn color(&mut self) -> usize {
        let c = self.next_color;
        self.next_color += 1;
        c
    }
    pub fn push(&mut self, commit: Commit) -> Row {
        let existing = self.lanes.iter().position(|l| l.as_ref().is_some_and(|l| l.id == commit.id));
        let lane = existing.unwrap_or_else(|| self.free());
        let color = if commit.refs.iter().any(|r| r.kind == super::model::RefKind::Head) {
            0
        } else {
            existing.map(|i| self.lanes[i].as_ref().unwrap().color).unwrap_or_else(|| self.color())
        };
        let mut lanes = self.lanes.len().max(lane + 1);
        let mut edges = Vec::new();
        for (i, pending) in self.lanes.iter().enumerate() {
            if let Some(pending) = pending {
                edges.push(Edge {
                    from: i,
                    to: i,
                    top: 0,
                    bottom: if pending.id == commit.id { 1 } else { 2 },
                    color: pending.color,
                });
            }
        }
        if existing.is_some() {
            self.lanes[lane] = None;
        }
        for (index, parent) in commit.parents.iter().enumerate() {
            let found = self.lanes.iter().position(|l| l.as_ref().is_some_and(|l| l.id == *parent));
            let target = if let Some(found) = found {
                found
            } else {
                let target = if index == 0 && self.lanes.get(lane).is_none_or(Option::is_none) { lane } else { self.free() };
                let parent_color = if index == 0 { color } else { self.color() };
                self.insert(target, parent.clone(), parent_color);
                target
            };
            edges.push(Edge { from: lane, to: target, top: 1, bottom: 2, color: self.lanes[target].as_ref().unwrap().color });
        }
        lanes = lanes.max(self.lanes.len());
        while self.lanes.last().is_some_and(Option::is_none) {
            self.lanes.pop();
        }
        Row { commit, lane, color, lanes, edges }
    }
}
