use anyhow::{Context, Result, bail};
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    pub path: PathBuf,
    pub original: Option<PathBuf>,
    pub index: char,
    pub worktree: char,
    pub conflicted: bool,
    pub submodule: bool,
}
impl FileStatus {
    pub fn kind(&self) -> &'static str {
        if self.conflicted {
            "Conflict"
        } else if self.index == '?' {
            "Untracked"
        } else if self.index != '.' && self.worktree != '.' {
            "Staged + unstaged"
        } else if self.index != '.' {
            "Staged"
        } else {
            "Unstaged"
        }
    }
}
impl FileStatus {
    /// A one-word `kind` for the file list's narrow first column.
    pub fn short_kind(&self) -> &'static str {
        if self.conflicted {
            "Conflict"
        } else if self.index == '?' {
            "New"
        } else if self.index != '.' && self.worktree != '.' {
            "Partial"
        } else if self.index != '.' {
            "Staged"
        } else if self.worktree == 'D' {
            "Deleted"
        } else {
            "Changed"
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub files: Vec<FileStatus>,
    pub upstream: Option<String>,
    pub divergence: Option<String>,
}
fn path(bytes: &[u8]) -> PathBuf {
    OsString::from_vec(bytes.to_vec()).into()
}

/// Parse NUL-delimited porcelain v2 without decoding paths as UTF-8. A rename's
/// second record is its original path, even if that path starts with '#'.
pub fn parse(bytes: &[u8]) -> Result<Status> {
    let mut result = Status::default();
    let mut records = bytes.split(|b| *b == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        if let Some(value) = record.strip_prefix(b"# branch.upstream ") {
            result.upstream = Some(String::from_utf8_lossy(value).into_owned());
            continue;
        }
        if let Some(value) = record.strip_prefix(b"# branch.ab ") {
            result.divergence = Some(String::from_utf8_lossy(value).into_owned());
            continue;
        }
        if record[0] == b'#' {
            continue;
        }
        if record.starts_with(b"? ") {
            result.files.push(FileStatus {
                path: path(&record[2..]),
                original: None,
                index: '?',
                worktree: '?',
                conflicted: false,
                submodule: false,
            });
            continue;
        }
        let fields = match record[0] {
            b'1' => 9,
            b'2' => 10,
            b'u' => 11,
            _ => bail!("Unknown Git status record"),
        };
        let parts: Vec<_> = record.splitn(fields, |b| *b == b' ').collect();
        if parts.len() != fields || parts[1].len() != 2 {
            bail!("Incomplete Git status record");
        }
        let original = if record[0] == b'2' { Some(path(records.next().context("Missing rename source")?)) } else { None };
        result.files.push(FileStatus {
            path: path(parts[fields - 1]),
            original,
            index: parts[1][0] as char,
            worktree: parts[1][1] as char,
            conflicted: record[0] == b'u',
            submodule: parts[2].starts_with(b"S"),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_renames_conflicts_and_non_utf8_paths() {
        let s = parse(b"# branch.upstream origin/main\0# branch.ab +2 -1\x002 R. N... 100644 100644 100644 a b R100 new name\0# old\0u UU N... 100644 100644 100644 100644 a b c conflict\0? raw\xff\0").unwrap();
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!(s.files[0].original, Some(PathBuf::from("# old")));
        assert_eq!(s.files[0].path, PathBuf::from("new name"));
        assert!(s.files[1].conflicted);
        assert_eq!(s.files[2].path, path(b"raw\xff"));
        assert!(parse(b"2 broken\0").is_err());
    }
}
