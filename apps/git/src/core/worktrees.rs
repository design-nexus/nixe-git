//! Native worktree management. Removal never uses force or deletes ignored data.
use super::{
    advanced::{self, Guard},
    operations, repository,
};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub locked: Option<String>,
    pub prunable: Option<String>,
    pub bare: bool,
    pub main: bool,
}
#[derive(Clone, Debug)]
pub enum State {
    Clean,
    Dirty(usize),
    Ignored,
    Missing,
    Unavailable(String),
    Bare,
}
#[derive(Clone, Debug)]
pub struct Overview {
    pub worktree: Worktree,
    pub state: State,
}
fn args(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}
fn git(root: &Path, label: &str, args: Vec<OsString>) -> Result<Vec<u8>> {
    operations::invoke(root, label, args, None, &[], false)
}
pub fn parse(bytes: &[u8]) -> Result<Vec<Worktree>> {
    let mut entries = Vec::new();
    let mut current: Option<Worktree> = None;
    for field in bytes.split(|b| *b == 0) {
        if field.is_empty() {
            if let Some(record) = current.take() {
                entries.push(record);
            }
            continue;
        }
        if let Some(path) = field.strip_prefix(b"worktree ") {
            ensure!(current.is_none(), "Malformed worktree listing");
            current = Some(Worktree {
                path: OsString::from_vec(path.to_vec()).into(),
                head: None,
                branch: None,
                locked: None,
                prunable: None,
                bare: false,
                main: entries.is_empty(),
            });
            continue;
        }
        let record = current.as_mut().context("Missing worktree path")?;
        if let Some(id) = field.strip_prefix(b"HEAD ") {
            let id = std::str::from_utf8(id)?;
            operations::valid_id(id)?;
            record.head = Some(id.into());
        } else if let Some(branch) = field.strip_prefix(b"branch ") {
            record.branch = Some(String::from_utf8(branch.to_vec())?);
        } else if field == b"bare" {
            record.bare = true;
        } else if field == b"locked" {
            record.locked = Some(String::new());
        } else if let Some(reason) = field.strip_prefix(b"locked ") {
            record.locked = Some(String::from_utf8_lossy(reason).into());
        } else if field == b"prunable" {
            record.prunable = Some(String::new());
        } else if let Some(reason) = field.strip_prefix(b"prunable ") {
            record.prunable = Some(String::from_utf8_lossy(reason).into());
        }
    }
    if let Some(record) = current {
        entries.push(record);
    }
    Ok(entries)
}
pub fn list(root: &Path) -> Result<Vec<Worktree>> {
    parse(&git(root, "List worktrees", args(&["worktree", "list", "--porcelain", "-z"]))?)
}
fn state(record: &Worktree) -> State {
    if record.bare {
        return State::Bare;
    }
    if !record.path.exists() {
        return State::Missing;
    }
    let result = (|| -> Result<_> {
        let repo = repository::inspect(&record.path)?;
        ensure!(repo.root == record.path.canonicalize()?, "This directory is no longer the registered worktree");
        let bytes = git(
            &record.path,
            "Read worktree state",
            args(&["--no-optional-locks", "status", "--porcelain=v2", "-z", "--untracked-files=all", "--ignored=matching"]),
        )?;
        // Ignored paths cannot be parsed as regular status records, and are never safe to delete silently.
        let ignored = bytes.split(|b| *b == 0).any(|r| r.starts_with(b"! "));
        let status = super::status::parse(&operations::run(&record.path, operations::Operation::Status)?)?;
        Ok(if !status.files.is_empty() {
            State::Dirty(status.files.len())
        } else if ignored {
            State::Ignored
        } else {
            State::Clean
        })
    })();
    result.unwrap_or_else(|e| State::Unavailable(format!("{e:#}")))
}
pub fn overview(root: &Path) -> Result<Vec<Overview>> {
    Ok(list(root)?
        .into_iter()
        .map(|worktree| {
            let state = state(&worktree);
            Overview { worktree, state }
        })
        .collect())
}
#[derive(Clone, Debug)]
pub enum Action {
    Create { path: PathBuf, branch: String, new_branch: bool, base: String },
    Remove { expected: Worktree, guard: Box<Guard> },
    Prune { expected: Vec<Worktree> },
    Lock { expected: Worktree, reason: String },
    Unlock { expected: Worktree },
}
impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Create { .. } => "Create worktree",
            Self::Remove { .. } => "Remove worktree",
            Self::Prune { .. } => "Prune stale worktrees",
            Self::Lock { .. } => "Lock worktree",
            Self::Unlock { .. } => "Unlock worktree",
        }
    }
}
pub fn removal_guard(root: &Path, expected: &Worktree) -> Result<Guard> {
    let records = list(root)?;
    let fresh = records.iter().find(|w| w.path == expected.path).context("Worktree is no longer registered")?;
    ensure!(fresh == expected, "The worktree changed; refresh the manager");
    ensure!(!fresh.main && !fresh.bare, "The main worktree cannot be removed");
    ensure!(
        fresh.path.canonicalize()? != repository::inspect(root)?.root,
        "Switch to another worktree before removing the active worktree"
    );
    ensure!(fresh.locked.is_none(), "Unlock this worktree before removing it");
    ensure!(
        matches!(state(fresh), State::Clean),
        "This worktree has changes, ignored files, is missing or cannot be inspected. Preserve its contents before removing it."
    );
    let guard = Guard::capture(&fresh.path)?;
    ensure!(guard.progress.is_none(), "Finish or abort the worktree's active Git operation first");
    guard.clean()?;
    Ok(guard)
}
pub fn run(root: &Path, action: Action) -> Result<Vec<u8>> {
    let inventory = list(root)?;
    match &action {
        Action::Create { path, branch, new_branch, base } => {
            advanced::valid_ref("refs/heads/", branch)?;
            operations::valid_id(base)?;
            ensure!(path.is_absolute(), "Choose an absolute worktree directory");
            ensure!(
                matches!(std::fs::symlink_metadata(path),Err(e)if e.kind()==std::io::ErrorKind::NotFound),
                "The destination already exists or cannot be inspected"
            );
            let parent = path.parent().context("Missing parent directory")?.canonicalize()?;
            let destination = parent.join(path.file_name().context("Missing directory name")?);
            ensure!(
                !inventory.iter().any(|w| destination.starts_with(&w.path)),
                "Create worktrees outside existing worktree directories"
            );
            ensure!(
                !inventory.iter().any(|w| w.branch.as_deref() == Some(&format!("refs/heads/{branch}"))),
                "This branch is already checked out in another worktree"
            );
            if !new_branch {
                ensure!(
                    advanced::resolve_target(root, &format!("refs/heads/{branch}"))? == *base,
                    "The branch moved; prepare creation again"
                );
            }
            let mut a = args(&["worktree", "add"]);
            if *new_branch {
                a.extend(args(&["-b", branch]));
            }
            a.push("--".into());
            a.push(destination.into_os_string());
            a.push(if *new_branch { base.into() } else { branch.into() });
            git(root, action.label(), a)
        }
        Action::Remove { expected, guard } => {
            guard.verify(&expected.path)?;
            removal_guard(root, expected)?;
            if let Some(head) = expected.head.as_deref() {
                advanced::checkpoint(root, head)?;
            }
            let mut a = args(&["worktree", "remove", "--"]);
            a.push(expected.path.as_os_str().into());
            git(root, action.label(), a)
        }
        Action::Prune { expected } => {
            ensure!(&inventory == expected, "The worktree list changed; review the prune preview again");
            ensure!(
                inventory.iter().filter(|w| w.prunable.is_some() && w.locked.is_none()).all(|w| !w.path.exists()),
                "A stale path exists again; inspect it before pruning"
            );
            for w in inventory.iter().filter(|w| w.prunable.is_some() && w.locked.is_none()) {
                if let Some(head) = w.head.as_deref() {
                    advanced::checkpoint(root, head)?;
                }
            }
            git(root, action.label(), args(&["worktree", "prune", "--verbose", "--expire=now"]))
        }
        Action::Lock { expected, .. } | Action::Unlock { expected } => {
            let fresh = inventory.iter().find(|w| w.path == expected.path).context("Worktree is no longer registered")?;
            ensure!(fresh == expected, "The worktree changed; refresh the manager");
            ensure!(!fresh.main, "The main worktree cannot be locked or unlocked");
            let mut a = if let Action::Lock { reason, .. } = &action {
                let mut a = args(&["worktree", "lock", "--reason"]);
                a.push(reason.clone().into());
                a
            } else {
                args(&["worktree", "unlock"])
            };
            a.push("--".into());
            a.push(expected.path.as_os_str().into());
            git(root, action.label(), a)
        }
    }
}
