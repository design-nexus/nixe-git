//! Bounded, pinned file history using Git's rename following and literal byte paths.
use super::{history, jobs, operations, repository};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

pub const PAGE_SIZE: usize = 50;
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub timestamp: i64,
    pub message: String,
    pub path: PathBuf,
    pub previous_path: Option<PathBuf>,
}
pub struct Page {
    pub anchor: Option<String>,
    pub entries: Vec<Entry>,
    pub older: bool,
}
fn check_cancel() -> Result<()> {
    ensure!(!jobs::context().is_some_and(|c| c.cancelled()), "File history loading stopped");
    Ok(())
}
pub fn page(root: &Path, file: &Path, anchor: Option<&str>, offset: usize, limit: usize) -> Result<Page> {
    ensure!((1..=PAGE_SIZE).contains(&limit), "Invalid history page size");
    let mut path_args = Vec::new();
    operations::paths(&mut path_args, &[file.into()])?;
    let anchor = match anchor {
        Some(id) => {
            operations::valid_id(id)?;
            Some(id.to_owned())
        }
        None => repository::inspect(root)?.head,
    };
    let Some(id) = anchor.as_ref() else {
        return Ok(Page { anchor, entries: Vec::new(), older: false });
    };
    let mut args: Vec<OsString> = [
        "--no-pager",
        "--literal-pathspecs",
        "log",
        "--follow",
        "--find-renames",
        "--root",
        "--no-show-signature",
        "--no-notes",
        "--no-decorate",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--format=%H",
        "--name-status",
        "-z",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    // Git --skip bypasses rename processing for skipped commits. Replay the prefix
    // and discard its records, retaining metadata only for the requested page.
    let count = offset.checked_add(limit + 1).context("History offset is too large")?;
    args.extend([format!("--max-count={count}").into(), id.into()]);
    args.extend(path_args);
    let bytes = operations::invoke(root, "Read file history", args, None, &[], false)?;
    let mut fields = bytes.split(|b| *b == 0).peekable();
    let mut seen = 0;
    let mut entries = Vec::new();
    while let Some(field) = fields.next() {
        check_cancel()?;
        if field.is_empty() {
            continue;
        }
        let id = std::str::from_utf8(field)?;
        operations::valid_id(id).context("Unexpected Git file-history record")?;
        let mut path = None;
        let mut previous_path = None;
        while let Some(raw) = fields.peek().copied() {
            if raw.is_empty() {
                fields.next();
                break;
            }
            let status = raw.strip_prefix(b"\n").unwrap_or(raw);
            if std::str::from_utf8(status).is_ok_and(|s| operations::valid_id(s).is_ok()) {
                break;
            }
            ensure!(
                matches!(status.first(), Some(b'A' | b'M' | b'D' | b'R' | b'C' | b'T' | b'U' | b'X' | b'B')),
                "Unexpected file change record"
            );
            fields.next();
            let first = fields.next().filter(|v| !v.is_empty()).context("Missing history path")?;
            if matches!(status.first(), Some(b'R' | b'C')) {
                let second = fields.next().filter(|v| !v.is_empty()).context("Missing renamed history path")?;
                previous_path = Some(PathBuf::from(OsString::from_vec(first.to_vec())));
                path = Some(PathBuf::from(OsString::from_vec(second.to_vec())));
            } else {
                path = Some(PathBuf::from(OsString::from_vec(first.to_vec())));
            }
        }
        seen += 1;
        if seen <= offset {
            continue;
        }
        let data = history::details(root, id)?;
        entries.push(Entry {
            id: id.into(),
            parents: data.parents,
            author: data.author.chars().take(256).collect(),
            timestamp: data.timestamp,
            message: data.message.chars().take(16_384).collect(),
            path: path.unwrap_or_else(|| file.into()),
            previous_path,
        });
        if entries.len() > limit {
            break;
        }
    }
    let older = entries.len() > limit;
    entries.truncate(limit);
    Ok(Page { anchor, entries, older })
}
pub fn diff(root: &Path, entry: &Entry) -> Result<Vec<u8>> {
    operations::valid_id(&entry.id)?;
    let repo = gix::discover(root)?;
    ensure!(
        entry.parents.is_empty() || !repo.shallow_commits()?.is_some_and(|ids| ids.iter().any(|id| id.to_string() == entry.id)),
        "Shallow history boundary: fetch more history to inspect this change"
    );
    let mut args: Vec<OsString> = if let Some(parent) = entry.parents.first() {
        operations::valid_id(parent)?;
        vec!["diff".into(), parent.into(), entry.id.clone().into()]
    } else {
        vec!["show".into(), "--format=".into(), "--root".into(), entry.id.clone().into()]
    };
    args.extend(
        ["--no-ext-diff", "--no-textconv", "--no-color", "--find-renames", "--full-index"].into_iter().map(OsString::from),
    );
    let mut paths = vec![entry.path.clone()];
    if let Some(previous) = &entry.previous_path
        && *previous != entry.path
    {
        paths.push(previous.clone());
    }
    operations::paths(&mut args, &paths)?;
    args.insert(0, "--literal-pathspecs".into());
    args.insert(0, "--no-pager".into());
    operations::invoke(root, "Read historical file diff", args, None, &[], false)
}
