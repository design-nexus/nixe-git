//! Bounded watch plans use Git's tracked/nonignored paths instead of walking caches.
use super::{operations, repository::Repository};
use anyhow::Result;
use std::{collections::BTreeSet, os::unix::ffi::OsStringExt, path::PathBuf};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub directories: Vec<PathBuf>,
    pub fallback: bool,
}
pub fn plan(repo: &Repository) -> Result<Plan> {
    let mut dirs = BTreeSet::from([repo.git_dir.clone(), repo.common_dir.clone()]);
    let mut fallback = false;
    let refs = repo.common_dir.join("refs");
    let mut pending = vec![refs, repo.common_dir.join("worktrees")];
    let mut ref_count = 0;
    while let Some(dir) = pending.pop() {
        if ref_count >= 128 {
            fallback = true;
            break;
        }
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
            continue;
        }
        ref_count += 1;
        dirs.insert(dir.clone());
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            if e.file_type()?.is_dir() {
                pending.push(e.path());
            }
        }
    }
    if !repo.bare {
        dirs.insert(repo.root.clone());
        let args = ["--no-optional-locks", "ls-files", "--cached", "--others", "--exclude-standard", "-z"]
            .iter()
            .map(Into::into)
            .collect();
        let bytes = operations::invoke(&repo.root, "Prepare file monitoring", args, None, &[], false)?;
        for path in bytes.split(|b| *b == 0).filter(|p| !p.is_empty()) {
            let relative = PathBuf::from(std::ffi::OsString::from_vec(path.to_vec()));
            let mut parent = relative.parent();
            while let Some(p) = parent {
                if p.as_os_str().is_empty() {
                    break;
                }
                if dirs.len() >= 640 {
                    fallback = true;
                    break;
                }
                let absolute = repo.root.join(p);
                if std::fs::symlink_metadata(&absolute).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
                    && absolute.canonicalize().is_ok_and(|p| p.starts_with(&repo.root))
                {
                    dirs.insert(absolute);
                }
                parent = p.parent();
            }
            if dirs.len() >= 640 {
                fallback = true;
                break;
            }
        }
        // Submodule internals and unwatched/uncreated directories get a low-frequency fallback.
        fallback |= repo.root.join(".gitmodules").exists();
    }
    Ok(Plan { directories: dirs.into_iter().collect(), fallback })
}
