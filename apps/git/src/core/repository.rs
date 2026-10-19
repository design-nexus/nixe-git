use super::{
    operations::{self, Operation},
    status::{self, Status},
};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Repository {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub branch: String,
    pub head: Option<String>,
    pub bare: bool,
    pub unborn: bool,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub repository: Repository,
    pub status: Status,
    pub progress: Option<super::advanced::Active>,
    pub watch: super::watch::Plan,
}

pub fn inspect(path: &Path) -> Result<Repository> {
    let repo = gix::discover(path).with_context(|| format!("No accessible Git repository at {}", path.display()))?;
    let head = repo.head().context("Could not read HEAD")?;
    let branch = head.referent_name().map(|n| n.shorten().to_string()).unwrap_or_else(|| "Detached HEAD".into());
    Ok(Repository {
        root: repo.workdir().unwrap_or(repo.path()).canonicalize()?,
        git_dir: repo.path().canonicalize()?,
        common_dir: repo.common_dir().canonicalize()?,
        branch,
        head: head.id().map(|id| id.to_string()),
        bare: repo.workdir().is_none(),
        unborn: head.is_unborn(),
    })
}
pub fn load(path: &Path) -> Result<Snapshot> {
    load_reusing(path, None)
}
/// As `load`, keeping a watch plan from an earlier read of the same repository when no
/// directory has appeared or gone since: building one lists every tracked and untracked file.
pub fn load_reusing(path: &Path, plan: Option<super::watch::Plan>) -> Result<Snapshot> {
    let repository = inspect(path)?;
    let status =
        if repository.bare { Status::default() } else { status::parse(&operations::run(&repository.root, Operation::Status)?)? };
    let progress = super::advanced::active(&repository.git_dir);
    let watch = plan.filter(|p| p.directories.contains(&repository.root)).unwrap_or_else(|| {
        super::watch::plan(&repository).unwrap_or_else(|_| super::watch::Plan {
            directories: vec![repository.root.clone(), repository.git_dir.clone(), repository.common_dir.clone()],
            fallback: true,
        })
    });
    Ok(Snapshot { repository, status, progress, watch })
}

/// Inspect a folder and its immediate children. Never recursively scan a home
/// directory, follow symlinks, or mistake all descendants for the same repo.
pub fn discover(folder: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    if gix::open(folder).is_ok() {
        found.push(inspect(folder)?.root);
    }
    for entry in std::fs::read_dir(folder).context("Could not read discovery folder")? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || entry.file_name().as_encoded_bytes().starts_with(b".") {
            continue;
        }
        if gix::open(entry.path()).is_ok() {
            found.push(inspect(&entry.path())?.root);
        }
    }
    found.sort();
    found.dedup();
    Ok(found)
}
