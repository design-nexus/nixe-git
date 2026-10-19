//! Optional Git LFS integration; native filters, endpoints and credentials remain authoritative.
use super::{advanced::Guard, operations};
use anyhow::{Result, ensure};
use std::{
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Snapshot {
    pub root: PathBuf,
    pub files: String,
    pub patterns: String,
    pub remotes: Vec<String>,
    guard: Guard,
    config: Vec<u8>,
    attributes: Option<Vec<u8>>,
    lfs_config: Option<Vec<u8>>,
}
impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LfsSnapshot").field("root", &self.root).finish()
    }
}
#[derive(Clone, Debug)]
pub enum Action {
    Enable,
    Track(String),
    Untrack(String),
    Fetch(String),
    Checkout,
}
impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Enable => "Enable repository LFS",
            Self::Track(_) => "Track LFS pattern",
            Self::Untrack(_) => "Untrack LFS pattern",
            Self::Fetch(_) => "Fetch LFS objects",
            Self::Checkout => "Check out cached LFS objects",
        }
    }
}
fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    operations::invoke(root, "Git LFS", args.iter().map(OsString::from).collect(), None, &[], false)
}
fn declaration(root: &Path, name: &str) -> Result<Option<Vec<u8>>> {
    let path = root.join(name);
    match fs::symlink_metadata(&path) {
        Ok(meta) => {
            ensure!(meta.is_file() && !meta.file_type().is_symlink(), "{name} must be a regular file");
            let mut bytes = Vec::new();
            fs::File::open(path)?.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 1024 * 1024, "{name} exceeds 1 MiB; manage it with Git");
            Ok(Some(bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn config(root: &Path) -> Result<Vec<u8>> {
    git(root, &["config", "--null", "--show-origin", "--list"])
}
fn display(bytes: Vec<u8>) -> Result<String> {
    ensure!(bytes.len() <= 1024 * 1024, "LFS listing exceeds 1 MiB; inspect it with git lfs");
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
pub fn read(root: &Path) -> Result<Snapshot> {
    git(root, &["lfs", "version"])?;
    let guard = Guard::capture(root)?;
    let root = guard.root.clone();
    let attributes = declaration(&root, ".gitattributes")?;
    let lfs_config = declaration(&root, ".lfsconfig")?;
    let files = display(git(&root, &["lfs", "ls-files", "--long"])?)?;
    let patterns = display(git(&root, &["lfs", "track"])?)?;
    let remotes = String::from_utf8(git(&root, &["remote"])?)?.lines().map(str::to_owned).collect();
    guard.verify(&root)?;
    // Native LFS inspection initializes lfs.repositoryformatversion when absent.
    let config = config(&root)?;
    ensure!(
        declaration(&root, ".gitattributes")? == attributes && declaration(&root, ".lfsconfig")? == lfs_config,
        "LFS declarations changed; reopen Git LFS"
    );
    Ok(Snapshot { root, files, patterns, remotes, guard, config, attributes, lfs_config })
}
pub fn run(root: &Path, action: Action, expected: &Snapshot) -> Result<Vec<u8>> {
    ensure!(root == expected.root, "Repository changed; reopen Git LFS");
    expected.guard.verify(root)?;
    ensure!(
        config(root)? == expected.config
            && declaration(root, ".gitattributes")? == expected.attributes
            && declaration(root, ".lfsconfig")? == expected.lfs_config,
        "LFS configuration or attributes changed; reopen Git LFS"
    );
    ensure!(expected.guard.progress.is_none(), "Finish the active Git operation first");
    match action {
        Action::Enable => git(root, &["lfs", "install", "--local"]),
        Action::Checkout => git(root, &["lfs", "checkout"]),
        action @ (Action::Track(_) | Action::Untrack(_)) => {
            let (command, pattern) = match action {
                Action::Track(p) => ("track", p),
                Action::Untrack(p) => ("untrack", p),
                _ => unreachable!(),
            };
            ensure!(
                !pattern.trim().is_empty() && pattern.len() <= 4096 && !pattern.chars().any(char::is_control),
                "Enter one nonempty LFS pattern without control characters"
            );
            git(root, &["lfs", command, "--", &pattern])
        }
        Action::Fetch(remote) => {
            ensure!(expected.remotes.contains(&remote), "Remote changed; reopen Git LFS");
            let head =
                expected.guard.head.as_deref().ok_or_else(|| anyhow::anyhow!("Create a commit before fetching LFS objects"))?;
            git(root, &["-c", "lfs.fetchrecentalways=false", "lfs", "fetch", "--", &remote, head])
        }
    }
}
