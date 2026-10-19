//! Guarded native remote configuration; no network requests occur here.
use super::{advanced, hosting, operations};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Snapshot {
    pub root: PathBuf,
    pub remotes: Vec<Remote>,
    config: Vec<u8>,
    refs: Vec<u8>,
    local_branches: bool,
}
// Config can contain credentials from existing external configuration.
impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteSnapshot").field("root", &self.root).field("count", &self.remotes.len()).finish()
    }
}
#[derive(Clone)]
pub struct Remote {
    pub name: String,
    pub fetch: Vec<String>,
    pub push: Vec<String>,
    pub editable: bool,
    pub remove_safe: bool,
}
#[derive(Clone, Debug)]
pub enum Action {
    Add { name: String, url: String },
    Rename { name: String, new: String },
    FetchUrl { name: String, url: String },
    PushUrl { name: String, url: Option<String> },
    Remove { name: String },
}
impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Add { .. } => "Add remote",
            Self::Rename { .. } => "Rename remote",
            Self::FetchUrl { .. } => "Change fetch URL",
            Self::PushUrl { .. } => "Change push URL",
            Self::Remove { .. } => "Remove remote",
        }
    }
}
fn git(root: &Path, label: &str, args: &[&str]) -> Result<Vec<u8>> {
    operations::invoke(root, label, args.iter().map(OsString::from).collect(), None, &[], false)
}
pub fn read(root: &Path) -> Result<Snapshot> {
    let repo = super::repository::inspect(root)?;
    let primary = repo.common_dir.join("config").canonicalize()?;
    let config = git(&repo.root, "Read remote configuration", &["config", "--null", "--show-origin", "--list"])?;
    let names = operations::run(&repo.root, operations::Operation::Remotes)?;
    let mut remotes = std::str::from_utf8(&names)
        .context("Remote names must be UTF-8")?
        .lines()
        .map(|name| Remote { name: name.into(), fetch: Vec::new(), push: Vec::new(), editable: true, remove_safe: true })
        .collect::<Vec<_>>();
    let records = config.split(|b| *b == 0).collect::<Vec<_>>();
    let mut local_branches = true;
    for record in records.as_chunks::<2>().0 {
        let (key, value) = split_byte(record[1], b'\n').unwrap_or((record[1], b""));
        let local = record[0]
            .strip_prefix(b"file:")
            .is_some_and(|path| repo.root.join(OsStr::from_bytes(path)).canonicalize().is_ok_and(|p| p == primary));
        let Ok(key) = std::str::from_utf8(key) else { continue };
        if key.starts_with("branch.") && [".remote", ".merge", ".pushremote"].iter().any(|suffix| key.ends_with(suffix)) && !local
        {
            local_branches = false;
        }
        let Some((name, field)) = key.strip_prefix("remote.").and_then(|s| s.rsplit_once('.')) else { continue };
        let Some(remote) = remotes.iter_mut().find(|r| r.name == name) else { continue };
        remote.editable &= local;
        if field == "fetch" && value != format!("+refs/heads/*:refs/remotes/{name}/*").as_bytes() {
            remote.remove_safe = false;
        }
        if field == "mirror" && ![b"false".as_slice(), b"no", b"0", b"off"].contains(&value) {
            remote.remove_safe = false;
        }
        if matches!(field, "url" | "pushurl") {
            let value = String::from_utf8(value.to_vec()).context("Remote URLs must be UTF-8 to manage them here")?;
            if field == "url" {
                remote.fetch.push(value);
            } else {
                remote.push.push(value);
            }
        }
    }
    let refs =
        git(&repo.root, "Read remote tracking tips", &["for-each-ref", "--format=%(refname)%00%(objectname)", "refs/remotes/"])?;
    Ok(Snapshot { root: repo.root, remotes, config, refs, local_branches })
}
pub fn validate_url(value: &str) -> Result<()> {
    ensure!(!value.is_empty() && !value.chars().any(char::is_control), "Enter a remote URL without control characters");
    if Path::new(value).is_absolute() {
        return Ok(());
    }
    hosting::transport(value).map(|_| ())
}
pub fn display_url(value: &str) -> String {
    if validate_url(value).is_ok() {
        value.into()
    } else {
        "URL hidden · use Git to inspect unsupported or credential-bearing URLs".into()
    }
}
fn valid_name(root: &Path, name: &str) -> Result<()> {
    ensure!(!name.starts_with('-') && !name.is_empty() && name != ".", "Enter a valid remote name");
    git(root, "Validate remote name", &["check-ref-format", &format!("refs/remotes/{name}/nixe-check")]).map(|_| ())
}
pub fn run(root: &Path, action: Action, expected: &Snapshot) -> Result<Vec<u8>> {
    let actual = read(root)?;
    ensure!(
        actual.root == expected.root && actual.config == expected.config && actual.refs == expected.refs,
        "Repository configuration or remote tracking refs changed. Reopen Remotes before trying again."
    );
    if let Action::Add { name, url } = &action {
        valid_name(root, name)?;
        validate_url(url)?;
        ensure!(!actual.remotes.iter().any(|r| r.name == *name), "A remote with this name already exists");
        return git(root, action.label(), &["remote", "add", "--", name, url]);
    }
    let name = match &action {
        Action::Rename { name, .. } | Action::FetchUrl { name, .. } | Action::PushUrl { name, .. } | Action::Remove { name } => {
            name
        }
        Action::Add { .. } => unreachable!(),
    };
    let remote = actual.remotes.iter().find(|r| r.name == *name).context("The remote no longer exists")?;
    ensure!(remote.editable, "This remote uses inherited or included configuration. Manage it with Git.");
    match &action {
        Action::Rename { new, .. } => {
            ensure!(actual.local_branches, "Branch tracking uses inherited configuration. Rename this remote with Git.");
            valid_name(root, new)?;
            ensure!(!actual.remotes.iter().any(|r| r.name == *new), "A remote with this name already exists");
            git(root, action.label(), &["remote", "rename", "--", name, new])
        }
        Action::FetchUrl { url, .. } => {
            ensure!(remote.fetch.len() == 1, "This remote has multiple or missing fetch URLs. Manage it with Git.");
            validate_url(url)?;
            git(root, action.label(), &["remote", "set-url", "--", name, url])
        }
        Action::PushUrl { url, .. } => {
            ensure!(remote.push.len() <= 1, "This remote has multiple push URLs. Manage it with Git.");
            if let Some(url) = url {
                validate_url(url)?;
                git(root, action.label(), &["remote", "set-url", "--push", "--", name, url])
            } else if remote.push.is_empty() {
                Ok(Vec::new())
            } else {
                git(root, action.label(), &["remote", "set-url", "--push", "--delete", "--", name, ".*"])
            }
        }
        Action::Remove { .. } => {
            ensure!(
                remote.remove_safe,
                "This remote uses mirror or custom fetch mappings. Remove it with Git after preserving its refs."
            );
            ensure!(actual.local_branches, "Branch tracking uses inherited configuration. Remove this remote with Git.");
            let prefix = format!("refs/remotes/{name}/");
            for line in actual.refs.split(|b| *b == b'\n') {
                let Some((reference, id)) = split_byte(line, 0) else { continue };
                if reference.starts_with(prefix.as_bytes()) {
                    advanced::checkpoint(root, std::str::from_utf8(id)?)?;
                }
            }
            // Check again after checkpoint writes, before removing native tracking refs.
            let fresh = read(root)?;
            ensure!(fresh.config == actual.config && fresh.refs == actual.refs, "Remote state changed; reopen Remotes");
            git(root, action.label(), &["remote", "remove", "--", name])
        }
        Action::Add { .. } => unreachable!(),
    }
}

fn split_byte(value: &[u8], byte: u8) -> Option<(&[u8], &[u8])> {
    let i = value.iter().position(|b| *b == byte)?;
    Some((&value[..i], &value[i + 1..]))
}

#[derive(Clone, Debug)]
pub struct RemoteBranch {
    pub name: String,
    pub id: String,
}
#[derive(Clone, Debug)]
pub struct BranchListing {
    pub snapshot: Snapshot,
    pub remote: String,
    endpoint: String,
    pub default: Option<String>,
    pub branches: Vec<RemoteBranch>,
}
fn push_endpoint(root: &Path, remote: &str) -> Result<String> {
    let bytes = git(root, "Read push destination", &["remote", "get-url", "--push", "--all", "--", remote])?;
    let urls = std::str::from_utf8(&bytes)?.lines().collect::<Vec<_>>();
    ensure!(urls.len() == 1, "Branch deletion requires exactly one push destination");
    validate_url(urls[0])?;
    Ok(urls[0].into())
}
pub fn branches(root: &Path, remote: &str) -> Result<BranchListing> {
    let snapshot = read(root)?;
    ensure!(snapshot.remotes.iter().any(|r| r.name == remote), "The remote no longer exists");
    let endpoint = push_endpoint(root, remote)?;
    let bytes = git(root, "Read server branches", &["ls-remote", "--symref", "--", &endpoint, "HEAD", "refs/heads/*"])?;
    let mut default = None;
    let mut branches = Vec::new();
    for line in std::str::from_utf8(&bytes)?.lines() {
        let Some((id, reference)) = line.split_once('\t') else { continue };
        if reference == "HEAD" {
            if let Some(name) = id.strip_prefix("ref: refs/heads/") {
                default = Some(name.into());
            }
        } else if let Some(name) = reference.strip_prefix("refs/heads/") {
            advanced::valid_ref("refs/heads/", name)?;
            operations::valid_id(id)?;
            branches.push(RemoteBranch { name: name.into(), id: id.into() });
        }
    }
    ensure!(read(root)?.config == snapshot.config, "Remote configuration changed; reopen server branches");
    Ok(BranchListing { snapshot, remote: remote.into(), endpoint, default, branches })
}
pub fn delete_branch(root: &Path, listing: &BranchListing, branch: &str, expected: &str) -> Result<Vec<u8>> {
    advanced::valid_ref("refs/heads/", branch)?;
    operations::valid_id(expected)?;
    ensure!(listing.branches.iter().any(|b| b.name == branch && b.id == expected), "Choose a branch from the server listing");
    let actual = read(root)?;
    ensure!(
        actual.root == listing.snapshot.root && actual.config == listing.snapshot.config && actual.refs == listing.snapshot.refs,
        "Repository configuration or tracking refs changed. Refresh server branches before deleting."
    );
    ensure!(push_endpoint(root, &listing.remote)? == listing.endpoint, "The push destination changed");
    let current = branches(root, &listing.remote)?;
    ensure!(
        current.default.as_deref().is_some_and(|default| default != branch),
        "The default branch cannot be deleted here; its identity must be advertised by the server"
    );
    ensure!(
        current.branches.iter().any(|b| b.name == branch && b.id == expected),
        "The server branch changed; refresh before deleting"
    );
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let recovery = format!("refs/nixe/recovery/{stamp}-{}", std::process::id());
    let reference = format!("refs/heads/{branch}");
    // Fetch the actual server-only tip before deleting it. A changed tip aborts deletion.
    git(
        root,
        "Retain remote branch tip",
        &["fetch", "--no-tags", "--no-recurse-submodules", "--", &listing.endpoint, &format!("{reference}:{recovery}")],
    )?;
    let saved = git(root, "Verify remote branch recovery", &["rev-parse", "--verify", &recovery])?;
    ensure!(
        std::str::from_utf8(&saved)?.trim() == expected,
        "The branch changed while fetching. Deletion stopped; the fetched tip remains in Recovery."
    );
    ensure!(
        read(root)?.config == listing.snapshot.config && push_endpoint(root, &listing.remote)? == listing.endpoint,
        "Remote configuration changed. Deletion stopped; the branch tip remains in Recovery."
    );
    git(root, "Delete remote branch", &["push", &format!("--force-with-lease={reference}:{expected}"), "--", &listing.endpoint, &format!(":{reference}")])
        .with_context(|| format!("Deletion did not complete successfully. The fetched tip is retained at {recovery}; refresh server branches before retrying"))
}
