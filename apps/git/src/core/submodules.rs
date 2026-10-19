//! Direct submodules, through native Git with fresh-state and local-work safeguards.
use super::{
    advanced::{self, Guard},
    operations, repository, status,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::Read,
    os::unix::ffi::OsStringExt,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Uninitialized,
    Clean,
    Dirty(usize),
    Conflict,
    Unavailable(String),
}
#[derive(Clone)]
pub struct Module {
    pub path: PathBuf,
    pub name: Option<String>,
    pub url: Option<String>,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub branch: Option<String>,
    pub state: State,
    child: Option<Guard>,
    child_config: Vec<u8>,
}
#[derive(Clone)]
pub struct Snapshot {
    pub root: PathBuf,
    pub modules: Vec<Module>,
    parent: Guard,
    configuration: Vec<u8>,
    declaration: Option<Vec<u8>>,
}
impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubmoduleSnapshot").field("root", &self.root).field("count", &self.modules.len()).finish()
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Action {
    Update,
    Sync,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Update => "Initialize/update submodule",
            Self::Sync => "Synchronize submodule URL",
        }
    }
}
fn git(root: &Path, label: &str, args: Vec<OsString>) -> Result<Vec<u8>> {
    operations::invoke(root, label, args, None, &[], false)
}
fn strings(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}
fn configuration(root: &Path) -> Result<Vec<u8>> {
    git(root, "Read submodule configuration", strings(&["config", "--null", "--show-origin", "--list"]))
}
fn declaration(root: &Path) -> Result<Option<Vec<u8>>> {
    let path = root.join(".gitmodules");
    match fs::symlink_metadata(&path) {
        Ok(m) => {
            ensure!(m.is_file() && !m.file_type().is_symlink(), ".gitmodules must be a regular file");
            let mut bytes = Vec::new();
            fs::File::open(path)?.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 1024 * 1024, ".gitmodules exceeds 1 MiB; inspect it with Git");
            Ok(Some(bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn safe_path(root: &Path, path: &Path) -> Result<PathBuf> {
    ensure!(
        !path.as_os_str().is_empty() && path.components().all(|c| matches!(c,Component::Normal(n)if n!=".git")),
        "Invalid submodule path"
    );
    let mut full = root.to_path_buf();
    for part in path.components() {
        full.push(part);
        match fs::symlink_metadata(&full) {
            Ok(m) => ensure!(m.is_dir() && !m.file_type().is_symlink(), "Submodule path contains a file or symbolic link"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(full)
}
pub fn read(root: &Path) -> Result<Snapshot> {
    let parent = Guard::capture(root)?;
    let root = parent.root.clone();
    let declaration = declaration(&root)?;
    let parent_configuration = configuration(&root)?;
    let mut definitions: BTreeMap<String, (Option<PathBuf>, Option<String>)> = BTreeMap::new();
    if declaration.is_some() {
        let bytes = git(
            &root,
            "Read submodule declarations",
            vec![
                "config".into(),
                "--null".into(),
                "--no-includes".into(),
                "--file".into(),
                root.join(".gitmodules").into_os_string(),
                "--list".into(),
            ],
        )?;
        for record in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let Some(i) = record.iter().position(|b| *b == b'\n') else { continue };
            let key = std::str::from_utf8(&record[..i])?;
            let value = &record[i + 1..];
            let Some((name, field)) = key.strip_prefix("submodule.").and_then(|k| k.rsplit_once('.')) else { continue };
            let entry = definitions.entry(name.into()).or_default();
            match field {
                "path" => entry.0 = Some(OsString::from_vec(value.to_vec()).into()),
                "url" => entry.1 = Some(String::from_utf8(value.to_vec()).context("Submodule URL must be UTF-8")?),
                _ => {}
            }
        }
    }
    let bytes = git(&root, "Read indexed submodules", strings(&["--no-optional-locks", "ls-files", "--stage", "-z"]))?;
    let mut links: BTreeMap<PathBuf, (Option<String>, bool)> = BTreeMap::new();
    for record in bytes.split(|b| *b == 0).filter(|r| r.starts_with(b"160000 ")) {
        let i = record.iter().position(|b| *b == b'\t').context("Malformed gitlink record")?;
        let fields = std::str::from_utf8(&record[..i])?.split_whitespace().collect::<Vec<_>>();
        ensure!(fields.len() == 3, "Malformed gitlink header");
        operations::valid_id(fields[1])?;
        let path: PathBuf = OsString::from_vec(record[i + 1..].to_vec()).into();
        let entry = links.entry(path).or_default();
        if fields[2] == "0" {
            entry.0 = Some(fields[1].into())
        } else {
            entry.1 = true;
        }
    }
    ensure!(links.len() <= 256, "More than 256 direct submodules; manage this repository with Git");
    let mut modules = Vec::new();
    for (path, (expected, conflict)) in links {
        let matches = definitions.iter().filter(|(_, d)| d.0.as_ref() == Some(&path)).collect::<Vec<_>>();
        let (name, url) = if matches.len() == 1 { (Some(matches[0].0.clone()), matches[0].1.1.clone()) } else { (None, None) };
        let mut module = Module {
            path,
            name,
            url,
            expected,
            actual: None,
            branch: None,
            state: State::Uninitialized,
            child: None,
            child_config: Vec::new(),
        };
        let result = (|| -> Result<()> {
            let full = safe_path(&root, &module.path)?;
            if conflict {
                module.state = State::Conflict;
                return Ok(());
            }
            ensure!(module.name.is_some() && module.url.is_some(), "Missing or ambiguous .gitmodules path/URL definition");
            if fs::symlink_metadata(full.join(".git")).is_ok() {
                let repo = repository::inspect(&full)?;
                ensure!(repo.root == full.canonicalize()?, "Submodule metadata resolves to another working directory");
                let guard = Guard::capture(&full)?;
                let parsed = status::parse(&guard.status)?;
                module.actual = guard.head.clone();
                module.branch = Some(guard.branch.clone());
                module.state = if parsed.files.is_empty() { State::Clean } else { State::Dirty(parsed.files.len()) };
                module.child_config = configuration(&full)?;
                module.child = Some(guard);
            }
            Ok(())
        })();
        if let Err(e) = result {
            module.state = State::Unavailable(format!("{e:#}"));
        }
        modules.push(module);
    }
    Ok(Snapshot { root, modules, parent, configuration: parent_configuration, declaration })
}
pub fn open_path(snapshot: &Snapshot, index: usize) -> Result<PathBuf> {
    let module = snapshot.modules.get(index).context("Choose a submodule")?;
    ensure!(module.child.is_some(), "Initialize this submodule before opening it");
    let full = safe_path(&snapshot.root, &module.path)?;
    ensure!(repository::inspect(&full)?.root == full.canonicalize()?, "The submodule is no longer initialized");
    Ok(full)
}
pub fn run(root: &Path, action: Action, index: usize, expected: &Snapshot) -> Result<Vec<u8>> {
    expected.parent.verify(root)?;
    ensure!(
        declaration(root)? == expected.declaration && configuration(root)? == expected.configuration,
        "Submodule configuration changed. Reopen Submodules before trying again."
    );
    ensure!(expected.parent.progress.is_none(), "Finish the parent repository’s active Git operation first");
    let module = expected.modules.get(index).context("Choose a submodule")?;
    ensure!(!matches!(module.state, State::Conflict | State::Unavailable(_)), "Resolve submodule metadata or conflicts first");
    let full = safe_path(root, &module.path)?;
    let work = || -> Result<Vec<u8>> {
        if let Some(guard) = &module.child {
            guard.verify(&full)?;
            ensure!(configuration(&full)? == module.child_config, "Submodule Git configuration changed; reopen Submodules");
            if matches!(action, Action::Update) {
                guard.clean()?;
                ensure!(guard.progress.is_none(), "Finish the submodule’s active Git operation first");
                let ignored = git(
                    &full,
                    "Check ignored submodule contents",
                    strings(&["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z"]),
                )?;
                ensure!(
                    ignored.is_empty(),
                    "Ignored files are present in the submodule. Move them aside or update with Git to avoid overwriting local contents."
                );
                if guard.head != module.expected
                    && let Some(id) = &guard.head
                {
                    advanced::checkpoint(&full, id)?;
                }
            }
        } else {
            ensure!(fs::symlink_metadata(full.join(".git")).is_err(), "Submodule initialization changed; reopen Submodules");
            if matches!(action, Action::Update) && full.exists() {
                ensure!(
                    fs::read_dir(&full)?.next().is_none(),
                    "The uninitialized submodule directory is not empty; inspect it before updating"
                );
            }
        }
        let mut args = strings(match action {
            Action::Update => &["submodule", "update", "--init", "--checkout"],
            Action::Sync => &["submodule", "sync"],
        });
        expected.parent.verify(root)?;
        ensure!(
            declaration(root)? == expected.declaration && configuration(root)? == expected.configuration,
            "Submodule configuration changed; reopen Submodules"
        );
        operations::paths(&mut args, std::slice::from_ref(&module.path))?;
        args.insert(0, "--literal-pathspecs".into());
        git(root,action.label(),args).context("Submodule operation failed. Partial initialization and local recovery refs are retained; inspect and refresh before retrying")
    };
    if module.child.is_some() { super::jobs::locked(&full, work) } else { work() }
}
