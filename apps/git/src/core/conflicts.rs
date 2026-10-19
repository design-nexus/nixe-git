//! Explicit, stale-state checked resolution of regular-file index conflicts.
use super::{advanced::Guard, operations};
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsString,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct Side {
    pub bytes: Vec<u8>,
    pub mode: u32,
}
#[derive(Clone, Debug)]
pub struct Conflict {
    pub root: PathBuf,
    pub file: PathBuf,
    pub base: Option<Side>,
    pub ours: Option<Side>,
    pub theirs: Option<Side>,
    pub working: Option<Vec<u8>>,
    guard: Guard,
    index: Vec<u8>,
}
#[derive(Clone, Debug)]
pub enum Resolution {
    Text(String),
    Ours,
    Theirs,
    Delete,
}
fn index(root: &Path, file: &Path) -> Result<Vec<u8>> {
    let mut args: Vec<OsString> = ["--literal-pathspecs", "ls-files", "--unmerged", "-z"].iter().map(OsString::from).collect();
    operations::paths(&mut args, &[file.to_path_buf()])?;
    operations::invoke(root, "Read conflict stages", args, None, &[], false)
}
fn safe_path(root: &Path, file: &Path) -> Result<PathBuf> {
    ensure!(
        !file.is_absolute() && file.components().all(|c| matches!(c, std::path::Component::Normal(_))),
        "Invalid repository path"
    );
    let path = root.join(file);
    let parent = path.parent().context("Missing parent")?.canonicalize()?;
    ensure!(parent.starts_with(root.canonicalize()?), "File parent leaves the working tree");
    if let Ok(m) = fs::symlink_metadata(&path) {
        ensure!(m.is_file() && !m.file_type().is_symlink(), "Resolve symlink and submodule conflicts with an external tool");
    }
    Ok(path)
}
pub fn load(root: &Path, file: &Path) -> Result<Conflict> {
    let guard = Guard::capture(root)?;
    let path = safe_path(&guard.root, file)?;
    let raw = index(&guard.root, file)?;
    ensure!(!raw.is_empty(), "This file is no longer conflicted");
    let repo = gix::discover(&guard.root)?;
    let mut sides: [Option<Side>; 3] = [None, None, None];
    for record in raw.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let header = record.split(|b| *b == b'\t').next().context("Invalid index record")?;
        let fields = std::str::from_utf8(header)?.split_whitespace().collect::<Vec<_>>();
        ensure!(fields.len() == 3, "Invalid conflict stage");
        let mode = u32::from_str_radix(fields[0], 8)?;
        ensure!(matches!(mode, 0o100644 | 0o100755), "Resolve symlink and submodule conflicts with an external tool");
        let stage: usize = fields[2].parse()?;
        ensure!((1..=3).contains(&stage), "Invalid conflict stage");
        let obj = repo.find_object(gix::ObjectId::from_hex(fields[1].as_bytes())?)?;
        ensure!(obj.data.len() <= 16 * 1024 * 1024, "Conflict exceeds the 16 MiB editor limit; use an external tool");
        sides[stage - 1] = Some(Side { bytes: obj.data.clone(), mode });
    }
    let working = match fs::read(path) {
        Ok(b) => {
            ensure!(b.len() <= 16 * 1024 * 1024, "Working file exceeds 16 MiB");
            Some(b)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let [base, ours, theirs] = sides;
    Ok(Conflict { root: guard.root.clone(), file: file.to_path_buf(), base, ours, theirs, working, guard, index: raw })
}
impl Conflict {
    pub fn text(&self) -> bool {
        self.base
            .iter()
            .chain(self.ours.iter())
            .chain(self.theirs.iter())
            .all(|s| !s.bytes.contains(&0) && std::str::from_utf8(&s.bytes).is_ok())
            && self.working.as_ref().is_none_or(|b| !b.contains(&0) && std::str::from_utf8(b).is_ok())
    }
}
pub fn has_markers(text: &str) -> bool {
    text.lines().any(|line| ["<<<<<<<", "=======", "|||||||", ">>>>>>>"].iter().any(|prefix| line.starts_with(prefix)))
}
pub fn resolve(root: &Path, conflict: &Conflict, resolution: Resolution) -> Result<Vec<u8>> {
    ensure!(root == conflict.root, "The active repository changed");
    conflict.guard.verify(root)?;
    ensure!(index(root, &conflict.file)? == conflict.index, "The conflict index changed; reopen the editor");
    let path = safe_path(root, &conflict.file)?;
    let now = match fs::read(&path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    ensure!(now == conflict.working, "The working file changed; reopen the editor");
    let side = match resolution {
        Resolution::Text(text) => {
            ensure!(!has_markers(&text), "Remove every conflict marker before marking this file resolved");
            Some(Side {
                bytes: text.into_bytes(),
                mode: conflict.ours.as_ref().or(conflict.theirs.as_ref()).map_or(0o100644, |s| s.mode),
            })
        }
        Resolution::Ours => conflict.ours.clone(),
        Resolution::Theirs => conflict.theirs.clone(),
        Resolution::Delete => None,
    };
    if let Some(side) = side {
        if let Ok(text) = std::str::from_utf8(&side.bytes) {
            ensure!(!has_markers(text), "The selected side contains conflict markers; edit the result before resolving");
        }
        let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("Missing parent")?)?;
        temp.write_all(&side.bytes)?;
        temp.as_file().set_permissions(fs::Permissions::from_mode(side.mode & 0o777))?;
        temp.persist(&path).map_err(|e| e.error)?;
    } else if path.exists() {
        fs::remove_file(&path)?;
    }
    let mut args: Vec<OsString> = ["--literal-pathspecs", "add", "--all"].iter().map(OsString::from).collect();
    operations::paths(&mut args, std::slice::from_ref(&conflict.file))?;
    match operations::invoke(root, "Mark conflict resolved", args, None, &[], false) {
        Ok(b) => Ok(b),
        Err(e) => bail!("The resolution was saved, but staging failed: {e:#}. Reopen the file to retry."),
    }
}
