//! Initialize without reinitializing an existing repository or staging user files.
use super::{advanced, operations};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn run(destination: &Path, branch: &str) -> Result<Vec<u8>> {
    advanced::valid_ref("refs/heads/", branch)?;
    ensure!(branch != "HEAD", "HEAD cannot be the initial branch name");
    ensure!(destination.is_absolute(), "Choose an absolute repository directory");
    let root: PathBuf = match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            ensure!(metadata.is_dir() && !metadata.file_type().is_symlink(), "Choose a directory, not a file or symbolic link");
            destination.canonicalize()?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = destination.parent().context("Choose a repository directory")?.canonicalize()?;
            ensure!(gix::discover(&parent).is_err(), "Choose a directory outside an existing repository");
            let name = destination.file_name().context("Choose a repository directory name")?;
            let path = parent.join(name);
            fs::create_dir(&path).context("Could not create the repository directory")?;
            path
        }
        Err(e) => return Err(e.into()),
    };
    ensure!(gix::discover(&root).is_err(), "This directory is already in a repository; open it instead");
    // Exclusive reservation also refuses broken repositories, .git files and concurrent init.
    fs::create_dir(root.join(".git"))
        .context("Git metadata already exists or cannot be created; inspect the directory before retrying")?;
    operations::invoke(
        &root,
        "Initialize repository",
        vec!["init".into(), format!("--initial-branch={branch}").into(), "--".into()],
        None,
        &[],
        false,
    )
    .context("Initialization failed. Directory contents and any partial Git metadata were retained for inspection")
}
