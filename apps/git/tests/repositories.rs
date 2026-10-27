use nyx_git::core::{
    operations::{self, Operation},
    repository,
};
use std::{ffi::OsStr, fs, path::Path, process::Command};
use tempfile::TempDir;

fn git(path: &Path, args: &[&OsStr]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
fn run(path: &Path, args: &[&str]) {
    git(path, &args.iter().map(OsStr::new).collect::<Vec<_>>());
}
fn fixture() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    run(dir.path(), &["init", "--initial-branch=main"]);
    run(dir.path(), &["config", "user.name", "Nyx Test"]);
    run(dir.path(), &["config", "user.email", "test@example.invalid"]);
    run(dir.path(), &["config", "commit.gpgsign", "false"]);
    run(dir.path(), &["config", "core.hooksPath", "/dev/null"]);
    run(dir.path(), &["config", "push.autoSetupRemote", "false"]);
    dir
}
fn commit(path: &Path) {
    run(path, &["add", "--all"]);
    run(path, &["commit", "-m", "Fixture"]);
}

#[test]
fn empty_status_special_paths_staging_rename_and_detached_head() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let dir = fixture();
    let root = dir.path();
    let empty = repository::load(root).unwrap();
    assert!(empty.repository.unborn);
    assert!(empty.status.files.is_empty());
    let paths =
        [OsString::from("space and 日本語.txt"), OsString::from("--option\nname"), OsString::from_vec(b"raw\xff".to_vec())];
    for name in &paths {
        fs::write(root.join(name), b"initial\n").unwrap();
    }
    fs::write(root.join(".gitignore"), "ignored\n").unwrap();
    fs::write(root.join("ignored"), "hidden").unwrap();
    let status = repository::load(root).unwrap().status;
    assert_eq!(status.files.len(), 4);
    assert!(status.files.iter().all(|f| f.kind() == "Untracked"));
    commit(root);
    fs::write(root.join(&paths[0]), b"staged\n").unwrap();
    git(root, &[OsStr::new("add"), OsStr::new("--"), &paths[0]]);
    fs::write(root.join(&paths[0]), b"unstaged too\n").unwrap();
    let status = repository::load(root).unwrap().status;
    assert_eq!(status.files[0].kind(), "Staged + unstaged");
    run(root, &["mv", "--", "--option\nname", "renamed file"]);
    let status = repository::load(root).unwrap().status;
    assert!(status.files.iter().any(|f| f.original.as_deref() == Some(Path::new("--option\nname"))));
    commit(root);
    run(root, &["checkout", "--detach"]);
    assert_eq!(repository::load(root).unwrap().repository.branch, "Detached HEAD");
    let before = fs::read(root.join(".git/index")).unwrap();
    operations::run(root, Operation::Status).unwrap();
    assert_eq!(before, fs::read(root.join(".git/index")).unwrap());
}
#[test]
fn linked_worktrees_bare_repositories_and_bounded_discovery() {
    let parent = tempfile::tempdir().unwrap();
    let repo = parent.path().join("repo");
    fs::create_dir(&repo).unwrap();
    run(&repo, &["init", "--initial-branch=main"]);
    run(&repo, &["config", "user.name", "Nyx Test"]);
    run(&repo, &["config", "user.email", "test@example.invalid"]);
    run(&repo, &["-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit", "--allow-empty", "-m", "Root"]);
    let worktree = parent.path().join("parallel");
    git(&repo, &[OsStr::new("worktree"), OsStr::new("add"), OsStr::new("-b"), OsStr::new("parallel"), worktree.as_os_str()]);
    fs::create_dir(worktree.join("nested")).unwrap();
    let snapshot = repository::load(&worktree.join("nested")).unwrap();
    assert_eq!(snapshot.repository.root, worktree);
    assert_eq!(snapshot.repository.branch, "parallel");
    let bare = parent.path().join("bare.git");
    fs::create_dir(&bare).unwrap();
    run(&bare, &["init", "--bare"]);
    assert!(repository::load(&bare).unwrap().repository.bare);
    assert_eq!(repository::discover(parent.path()).unwrap().len(), 3);
    assert!(repository::load(parent.path()).is_err());
}
#[test]
fn conflicts_are_reported_without_changing_working_files() {
    let dir = fixture();
    let root = dir.path();
    fs::write(root.join("file"), "base\n").unwrap();
    commit(root);
    run(root, &["checkout", "-b", "other"]);
    fs::write(root.join("file"), "theirs\n").unwrap();
    commit(root);
    run(root, &["checkout", "main"]);
    fs::write(root.join("file"), "ours\n").unwrap();
    commit(root);
    let result = Command::new("git").arg("-C").arg(root).args(["merge", "other"]).output().unwrap();
    assert!(!result.status.success());
    let before = fs::read(root.join("file")).unwrap();
    let status = repository::load(root).unwrap().status;
    assert_eq!(status.files.len(), 1);
    assert!(status.files[0].conflicted);
    assert_eq!(before, fs::read(root.join("file")).unwrap());
    assert!(operations::run(root, Operation::StageAll).is_err());
    assert_eq!(before, fs::read(root.join("file")).unwrap());
}

#[test]
fn mutations_commit_and_unstage_preserve_working_files() {
    let dir = fixture();
    let root = dir.path();
    fs::write(root.join("* literal"), "one\n").unwrap();
    fs::write(root.join("other"), "untouched\n").unwrap();
    operations::run(root, Operation::Stage(vec!["* literal".into()])).unwrap();
    let snapshot = repository::load(root).unwrap();
    assert_eq!(snapshot.status.files.iter().filter(|f| f.kind() == "Staged").count(), 1);
    fs::write(root.join("* literal"), "two\n").unwrap();
    operations::run(root, Operation::Unstage { paths: vec!["* literal".into()], unborn: true }).unwrap();
    assert_eq!(fs::read_to_string(root.join("* literal")).unwrap(), "two\n");
    operations::run(root, Operation::StageAll).unwrap();
    operations::run(root, Operation::UnstageAll { unborn: true }).unwrap();
    assert!(repository::load(root).unwrap().status.files.iter().all(|f| f.index == '?'));
    operations::run(root, Operation::StageAll).unwrap();
    operations::run(root, Operation::Commit { message: "First\n\nBody with `$(literal)`".into(), amend: false }).unwrap();
    let first = repository::load(root).unwrap().repository.head.unwrap();
    operations::run(root, Operation::Commit { message: "Amended".into(), amend: true }).unwrap();
    assert_ne!(first, repository::load(root).unwrap().repository.head.unwrap());
    fs::write(root.join("* literal"), "three\n").unwrap();
    operations::run(root, Operation::StageAll).unwrap();
    operations::run(root, Operation::UnstageAll { unborn: false }).unwrap();
    assert_eq!(repository::load(root).unwrap().status.files[0].kind(), "Unstaged");
    let untracked = root.join("new file");
    fs::write(&untracked, "preview\n").unwrap();
    let diff = operations::run(root, Operation::Diff { file: "new file".into(), cached: false, untracked: true }).unwrap();
    assert!(String::from_utf8_lossy(&diff).contains("+preview"));
    // A failed native hook must leave both the staged change and working file intact.
    use std::os::unix::fs::PermissionsExt;
    let hooks = root.join("hooks");
    fs::create_dir(&hooks).unwrap();
    fs::write(hooks.join("pre-commit"), "#!/bin/sh\necho 'Blocked by hook' >&2\nexit 1\n").unwrap();
    fs::set_permissions(hooks.join("pre-commit"), fs::Permissions::from_mode(0o755)).unwrap();
    git(root, &[OsStr::new("config"), OsStr::new("core.hooksPath"), hooks.as_os_str()]);
    operations::run(root, Operation::Stage(vec!["* literal".into()])).unwrap();
    assert!(
        operations::run(root, Operation::Commit { message: "Blocked".into(), amend: false })
            .unwrap_err()
            .to_string()
            .contains("Blocked by hook")
    );
    assert_eq!(fs::read_to_string(root.join("* literal")).unwrap(), "three\n");
    assert!(repository::load(root).unwrap().status.files.iter().any(|f| f.index == 'M'));
}
#[test]
fn stages_selected_lines_and_hunks_and_rejects_stale_patches() {
    use nyx_git::core::staging;
    let dir = fixture();
    let root = dir.path();
    let original: String = (1..=40).map(|i| format!("line {i}\n")).collect();
    fs::write(root.join("file"), &original).unwrap();
    fs::write(root.join("empty"), "").unwrap();
    commit(root);
    let changed = original.replace("line 2\n", "changed 2\n").replace("line 30\n", "changed 30\n");
    fs::write(root.join("file"), &changed).unwrap();
    let diff = || operations::run(root, Operation::Diff { file: "file".into(), cached: false, untracked: false }).unwrap();
    let bytes = diff();
    let text = std::str::from_utf8(&bytes).unwrap();
    let added = text.lines().position(|l| l == "+changed 2").unwrap();
    operations::run(root, Operation::StageSelection { file: "file".into(), expected: bytes, range: (added, added + 1) }).unwrap();
    let indexed = Command::new("git").arg("-C").arg(root).args(["show", ":file"]).output().unwrap().stdout;
    assert_eq!(String::from_utf8(indexed).unwrap(), original.replace("line 2\n", "line 2\nchanged 2\n"));
    assert_eq!(fs::read_to_string(root.join("file")).unwrap(), changed);
    operations::run(root, Operation::UnstageAll { unborn: false }).unwrap();
    let bytes = diff();
    let text = std::str::from_utf8(&bytes).unwrap();
    let line = text.lines().position(|l| l == "+changed 30").unwrap();
    let range = staging::hunk_range(&bytes, line).unwrap();
    operations::run(root, Operation::StageSelection { file: "file".into(), expected: bytes, range }).unwrap();
    let indexed = Command::new("git").arg("-C").arg(root).args(["show", ":file"]).output().unwrap().stdout;
    assert_eq!(String::from_utf8(indexed).unwrap(), original.replace("line 30\n", "changed 30\n"));
    let bytes = diff();
    let before = fs::read(root.join(".git/index")).unwrap();
    fs::write(root.join("file"), changed + "external edit\n").unwrap();
    assert!(operations::run(root, Operation::StageSelection { file: "file".into(), expected: bytes, range: (0, 1000) }).is_err());
    assert_eq!(before, fs::read(root.join(".git/index")).unwrap());
    fs::write(root.join("empty"), "first line\n").unwrap();
    let bytes = operations::run(root, Operation::Diff { file: "empty".into(), cached: false, untracked: false }).unwrap();
    operations::run(root, Operation::StageSelection { file: "empty".into(), expected: bytes, range: (0, 1000) }).unwrap();
    let indexed = Command::new("git").arg("-C").arg(root).args(["show", ":empty"]).output().unwrap().stdout;
    assert_eq!(indexed, b"first line\n");
}
#[test]
fn branches_stashes_and_local_remotes_use_native_git_safeguards() {
    let dir = fixture();
    let root = dir.path();
    fs::write(root.join("file"), "base\n").unwrap();
    commit(root);
    operations::run(root, Operation::CreateBranch("feature".into())).unwrap();
    operations::run(root, Operation::RenameBranch { old: "feature".into(), new: "renamed".into() }).unwrap();
    operations::run(root, Operation::SwitchBranch("main".into())).unwrap();
    operations::run(root, Operation::DeleteBranch("renamed".into())).unwrap();
    fs::write(root.join("file"), "saved\n").unwrap();
    fs::write(root.join("untracked"), "keep me\n").unwrap();
    operations::run(root, Operation::StashSave("Saved safely".into())).unwrap();
    assert_eq!(fs::read_to_string(root.join("file")).unwrap(), "base\n");
    assert!(!root.join("untracked").exists());
    let listing = operations::run(root, Operation::Stashes).unwrap();
    let hash = String::from_utf8_lossy(listing.split(|b| *b == 0).next().unwrap()).to_string();
    let preview = operations::run(root, Operation::StashDiff(hash.clone())).unwrap();
    assert!(String::from_utf8_lossy(&preview).contains("keep me"));
    operations::run(root, Operation::StashApply(hash.clone())).unwrap();
    assert_eq!(fs::read_to_string(root.join("untracked")).unwrap(), "keep me\n");
    assert!(operations::run(root, Operation::StashDrop { id: "stash@{0}".into(), expected: "outdated".into() }).is_err());
    operations::run(root, Operation::StashDrop { id: "stash@{0}".into(), expected: hash }).unwrap();
    assert!(operations::run(root, Operation::Stashes).unwrap().is_empty());
    operations::run(root, Operation::StashSave("Pop later".into())).unwrap();
    let hash =
        String::from_utf8_lossy(&operations::run(root, Operation::StashOid("stash@{0}".into())).unwrap()).trim().to_string();
    operations::run(root, Operation::StashPop { id: "stash@{0}".into(), expected: hash }).unwrap();
    operations::run(root, Operation::StageAll).unwrap();
    operations::run(root, Operation::Commit { message: "Restored".into(), amend: false }).unwrap();
    // All remote tests are local and cannot use real accounts or publish user work.
    let remote = tempfile::tempdir().unwrap();
    run(remote.path(), &["init", "--bare", "--initial-branch=main"]);
    git(root, &[OsStr::new("remote"), OsStr::new("add"), OsStr::new("origin"), remote.path().as_os_str()]);
    run(root, &["config", "push.default", "nothing"]);
    let error = operations::run(root, Operation::Push).unwrap_err().to_string();
    assert!(error.contains("push.default") && !error.contains("credential"));
    run(root, &["config", "push.default", "simple"]);
    operations::run(root, Operation::PushBranch { remote: "origin".into(), branch: "main".into() }).unwrap();
    operations::run(root, Operation::Push).unwrap();
    operations::run(root, Operation::Fetch).unwrap();
    operations::run(root, Operation::Pull).unwrap();
    operations::run(root, Operation::CreateBranch("unmerged".into())).unwrap();
    fs::write(root.join("unique"), "only on branch").unwrap();
    commit(root);
    operations::run(root, Operation::SwitchBranch("main".into())).unwrap();
    assert!(operations::run(root, Operation::DeleteBranch("unmerged".into())).is_err());
    assert!(String::from_utf8_lossy(&operations::run(root, Operation::Branches).unwrap()).contains("unmerged"));
}
