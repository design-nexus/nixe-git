//! Serializable workspace state; no GTK objects or Git mutations.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RepositoryState {
    pub root: PathBuf,
    pub favorite: bool,
    pub group: String,
    pub details_position: i32,
    pub narrow_position: i32,
    pub view: String,
    pub zoom: f64,
    pub message: String,
    pub amend: bool,
    pub sidebar_closed: Vec<String>,
}
impl Default for RepositoryState {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            favorite: false,
            group: String::new(),
            details_position: 560,
            narrow_position: 400,
            view: "history".into(),
            zoom: 1.,
            message: String::new(),
            amend: false,
            sidebar_closed: vec!["remote-branches".into(), "tags".into(), "stashes".into(), "worktrees".into(), "remotes".into()],
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub open: Vec<PathBuf>,
    pub active: Option<PathBuf>,
    pub repositories: Vec<RepositoryState>,
}
impl Workspace {
    pub fn entry(&mut self, root: &Path) -> &mut RepositoryState {
        if let Some(index) = self.repositories.iter().position(|r| r.root == root) {
            return &mut self.repositories[index];
        }
        self.repositories.push(RepositoryState { root: root.into(), ..Default::default() });
        self.repositories.last_mut().unwrap()
    }
    pub fn activate(&mut self, root: PathBuf) {
        if !self.open.contains(&root) {
            self.open.push(root.clone());
        }
        self.active = Some(root);
    }
    pub fn close(&mut self, root: &Path) -> Option<PathBuf> {
        if let Some(i) = self.open.iter().position(|p| p == root) {
            self.open.remove(i);
            if self.active.as_deref() == Some(root) {
                self.active = self.open.get(i.min(self.open.len().saturating_sub(1))).cloned();
            }
        }
        self.active.clone()
    }
    pub fn next(&self, backwards: bool) -> Option<PathBuf> {
        if self.open.is_empty() {
            return None;
        }
        let i = self.active.as_ref().and_then(|p| self.open.iter().position(|r| r == p)).unwrap_or(0);
        let i = if backwards { (i + self.open.len() - 1) % self.open.len() } else { (i + 1) % self.open.len() };
        Some(self.open[i].clone())
    }
}
