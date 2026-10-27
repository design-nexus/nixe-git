use nyx_git::core::{
    advanced,
    operations::{self, Operation},
    submodules::{self, Action, State},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().into()
}
fn init(root: &Path) {
    fs::create_dir(root).unwrap();
    git(root, &["init", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Submodule Test"),
        ("user.email", "submodule@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        git(root, &["config", k, v]);
    }
}
fn commit(root: &Path, msg: &str) {
    git(root, &["add", "--all"]);
    git(root, &["commit", "-m", msg]);
}
const PATH: &str = "deps/lib * 日本語";
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let parent = temp.path().join("parent");
    init(&source);
    fs::write(source.join("file"), "original\n").unwrap();
    commit(&source, "Base");
    init(&parent);
    git(&parent, &["-c", "protocol.file.allow=always", "submodule", "add", "--", source.to_str().unwrap(), PATH]);
    commit(&parent, "Add submodule");
    let child = parent.join(PATH);
    git(&child, &["config", "core.hooksPath", "/dev/null"]);
    (temp, parent, source, child)
}
fn action(root: &Path, action: Action, snapshot: submodules::Snapshot) -> anyhow::Result<Vec<u8>> {
    operations::run(root, Operation::Submodule { action, index: 0, expected: Box::new(snapshot) })
}
#[test]
fn indexed_checkout_retains_previous_child_tip_and_supports_cached_initialization_and_url_sync() {
    let (_temp, parent, source, child) = fixture();
    let other_path = "deps/lib x 日本語";
    git(&parent, &["-c", "protocol.file.allow=always", "submodule", "add", "--", source.to_str().unwrap(), other_path]);
    commit(&parent, "Add second submodule");
    let other = parent.join(other_path);
    fs::write(other.join("file"), "preserve other submodule\n").unwrap();
    let parent_head = git(&parent, &["rev-parse", "HEAD"]);
    let old = git(&child, &["rev-parse", "HEAD"]);
    fs::write(source.join("file"), "updated\n").unwrap();
    commit(&source, "Update source");
    let new = git(&source, &["rev-parse", "HEAD"]);
    git(&child, &["fetch", "origin"]);
    git(&parent, &["update-index", "--cacheinfo", "160000", &new, PATH]);
    git(&parent, &["update-index", "--cacheinfo", "160000", &new, other_path]);
    let index = git(&parent, &["ls-files", "--stage", "--", PATH]);
    let snapshot = submodules::read(&parent).unwrap();
    assert_eq!(snapshot.modules.len(), 2);
    assert_eq!(snapshot.modules[0].state, State::Clean);
    assert_eq!(snapshot.modules[0].expected.as_deref(), Some(new.as_str()));
    action(&parent, Action::Update, snapshot).unwrap();
    assert_eq!(git(&child, &["rev-parse", "HEAD"]), new);
    assert_eq!(fs::read_to_string(child.join("file")).unwrap(), "updated\n");
    assert_eq!(git(&parent, &["ls-files", "--stage", "--", PATH]), index);
    assert_eq!(git(&parent, &["rev-parse", "HEAD"]), parent_head);
    assert_eq!(git(&other, &["rev-parse", "HEAD"]), old);
    assert_eq!(fs::read_to_string(other.join("file")).unwrap(), "preserve other submodule\n");
    assert!(advanced::recovery(&child).unwrap().iter().any(|r| r.id == old && r.label.starts_with("refs/nyx/recovery/")));
    git(&parent, &["commit", "-m", "Record indexed submodule commits"]);
    git(&parent, &["--literal-pathspecs", "submodule", "deinit", "--", PATH]);
    let snapshot = submodules::read(&parent).unwrap();
    assert_eq!(snapshot.modules[0].state, State::Uninitialized);
    assert!(submodules::open_path(&snapshot, 0).is_err());
    action(&parent, Action::Update, snapshot).unwrap();
    assert_eq!(git(&child, &["rev-parse", "HEAD"]), new);
    let next_url = source.parent().unwrap().join("new source location");
    let key = format!("submodule.{PATH}.url");
    git(&parent, &["config", "--file", ".gitmodules", &key, next_url.to_str().unwrap()]);
    let snapshot = submodules::read(&parent).unwrap();
    assert_eq!(submodules::open_path(&snapshot, 0).unwrap(), child);
    action(&parent, Action::Sync, snapshot).unwrap();
    assert_eq!(git(&parent, &["config", &key]), next_url.to_str().unwrap());
    assert_eq!(git(&child, &["config", "remote.origin.url"]), next_url.to_str().unwrap());
}
#[test]
fn local_changes_ignored_files_stale_configuration_and_nonempty_uninitialized_directories_are_preserved() {
    let (_temp, parent, _source, child) = fixture();
    let head = git(&child, &["rev-parse", "HEAD"]);
    let parent_head = git(&parent, &["rev-parse", "HEAD"]);
    fs::write(child.join("file"), "uncommitted\n").unwrap();
    let snapshot = submodules::read(&parent).unwrap();
    assert!(matches!(snapshot.modules[0].state, State::Dirty(_)));
    assert!(action(&parent, Action::Update, snapshot).is_err());
    assert_eq!(fs::read_to_string(child.join("file")).unwrap(), "uncommitted\n");
    fs::write(child.join("file"), "original\n").unwrap();
    let info = PathBuf::from(git(&child, &["rev-parse", "--git-path", "info/exclude"]));
    let info = if info.is_absolute() { info } else { child.join(info) };
    fs::write(info, "ignored.txt\n").unwrap();
    fs::write(child.join("ignored.txt"), "keep ignored\n").unwrap();
    let snapshot = submodules::read(&parent).unwrap();
    assert_eq!(snapshot.modules[0].state, State::Clean);
    assert!(action(&parent, Action::Update, snapshot).unwrap_err().to_string().contains("Ignored"));
    assert_eq!(fs::read_to_string(child.join("ignored.txt")).unwrap(), "keep ignored\n");
    fs::remove_file(child.join("ignored.txt")).unwrap();
    let snapshot = submodules::read(&parent).unwrap();
    git(&parent, &["config", &format!("submodule.{PATH}.url"), "https://example.invalid/changed.git"]);
    assert!(action(&parent, Action::Update, snapshot).is_err());
    assert_eq!(git(&child, &["rev-parse", "HEAD"]), head);
    git(&parent, &["--literal-pathspecs", "submodule", "deinit", "--", PATH]);
    fs::write(child.join("keep"), "keep uninitialized\n").unwrap();
    let snapshot = submodules::read(&parent).unwrap();
    assert!(action(&parent, Action::Update, snapshot).is_err());
    assert_eq!(fs::read_to_string(child.join("keep")).unwrap(), "keep uninitialized\n");
    assert_eq!(git(&parent, &["rev-parse", "HEAD"]), parent_head);
}
