use nyx_git::core::{
    advanced::{self, Action, Guard, Pick, ResetMode},
    conflicts::{self, Resolution},
    repository,
};
use std::{fs, path::Path, process::Command};
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().into()
}
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    git(p, &["init", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
        ("rerere.enabled", "false"),
    ] {
        git(p, &["config", key, value]);
    }
    write_commit(p, "file", "base\n", "base");
    d
}
fn write_commit(root: &Path, file: &str, text: &str, msg: &str) {
    fs::write(root.join(file), text).unwrap();
    git(root, &["add", "--all"]);
    git(root, &["commit", "-m", msg]);
}
fn apply(root: &Path, a: Action) -> anyhow::Result<Vec<u8>> {
    let guard = Guard::capture(root)?;
    advanced::run(root, a, &guard)
}
fn conflict_fixture() -> (tempfile::TempDir, String) {
    let d = fixture();
    let p = d.path();
    git(p, &["checkout", "-b", "topic"]);
    write_commit(p, "file", "theirs\n", "topic");
    let id = git(p, &["rev-parse", "HEAD"]);
    git(p, &["checkout", "main"]);
    write_commit(p, "file", "ours\n", "main");
    (d, id)
}
#[test]
fn merge_conflict_editor_validation_stale_state_and_continue() {
    let (d, tip) = conflict_fixture();
    let p = d.path();
    assert!(apply(p, Action::Merge(tip)).is_err());
    assert_eq!(repository::load(p).unwrap().progress, Some(advanced::Active::Merge));
    assert!(apply(p, Action::Continue).is_err());
    let c = conflicts::load(p, Path::new("file")).unwrap();
    assert_eq!(c.base.as_ref().unwrap().bytes, b"base\n");
    assert!(conflicts::resolve(p, &c, Resolution::Text("<<<<<<< unresolved\n".into())).is_err());
    fs::write(p.join("file"), "external change\n").unwrap();
    assert!(conflicts::resolve(p, &c, Resolution::Ours).is_err());
    let c = conflicts::load(p, Path::new("file")).unwrap();
    conflicts::resolve(p, &c, Resolution::Text("combined\n".into())).unwrap();
    apply(p, Action::Continue).unwrap();
    assert!(git(p, &["status", "--porcelain"]).is_empty());
    assert_eq!(git(p, &["rev-list", "--parents", "-n", "1", "HEAD"]).split_whitespace().count(), 3);
    assert!(!advanced::recovery(p).unwrap().is_empty());
}
#[test]
fn rebase_cherry_pick_revert_abort_and_binary_resolution() {
    let (d, tip) = conflict_fixture();
    let p = d.path();
    let old = git(p, &["rev-parse", "HEAD"]);
    assert!(apply(p, Action::Rebase(tip.clone())).is_err());
    assert_eq!(repository::load(p).unwrap().progress, Some(advanced::Active::Rebase));
    let c = conflicts::load(p, Path::new("file")).unwrap();
    conflicts::resolve(p, &c, Resolution::Ours).unwrap();
    apply(p, Action::Continue).unwrap();
    assert_eq!(fs::read_to_string(p.join("file")).unwrap(), "theirs\n");
    apply(p, Action::Reset { target: old.clone(), mode: ResetMode::Hard }).unwrap();
    assert!(apply(p, Action::CherryPick { id: tip.clone(), mainline: None }).is_err());
    apply(p, Action::Abort).unwrap();
    assert_eq!(git(p, &["rev-parse", "HEAD"]), old);
    git(p, &["checkout", "topic"]);
    write_commit(p, "other", "new\n", "new");
    let pick = git(p, &["rev-parse", "HEAD"]);
    git(p, &["checkout", "main"]);
    apply(p, Action::CherryPick { id: pick.clone(), mainline: None }).unwrap();
    assert!(p.join("other").exists());
    apply(p, Action::Revert { id: pick, mainline: None }).unwrap();
    assert!(!p.join("other").exists());
    git(p, &["checkout", "-b", "binary-topic"]);
    fs::write(p.join("file"), b"theirs\0binary").unwrap();
    git(p, &["commit", "-am", "binary"]);
    let tip = git(p, &["rev-parse", "HEAD"]);
    git(p, &["checkout", "main"]);
    fs::write(p.join("file"), b"ours\0binary").unwrap();
    git(p, &["commit", "-am", "binary"]);
    assert!(apply(p, Action::Merge(tip)).is_err());
    let c = conflicts::load(p, Path::new("file")).unwrap();
    assert!(!c.text());
    conflicts::resolve(p, &c, Resolution::Theirs).unwrap();
    apply(p, Action::Continue).unwrap();
    assert_eq!(fs::read(p.join("file")).unwrap(), b"theirs\0binary");
}
#[test]
fn resets_recovery_tags_discard_and_guard() {
    let d = fixture();
    let p = d.path();
    let base = git(p, &["rev-parse", "HEAD"]);
    write_commit(p, "file", "second\n", "second");
    let tip = git(p, &["rev-parse", "HEAD"]);
    apply(p, Action::Reset { target: base.clone(), mode: ResetMode::Soft }).unwrap();
    let s = repository::load(p).unwrap();
    assert!(s.status.files.iter().any(|f| f.index == 'M'));
    assert!(advanced::recovery(p).unwrap().iter().any(|r| r.id == tip));
    apply(p, Action::Reset { target: tip.clone(), mode: ResetMode::Mixed }).unwrap();
    assert!(git(p, &["status", "--porcelain"]).is_empty());
    fs::write(p.join("file"), "local\n").unwrap();
    let guard = Guard::capture(p).unwrap();
    fs::write(p.join("file"), "changed again\n").unwrap();
    assert!(advanced::run(p, Action::Reset { target: base.clone(), mode: ResetMode::Hard }, &guard).is_err());
    assert_eq!(git(p, &["rev-parse", "HEAD"]), tip);
    apply(p, Action::Discard { file: "file".into(), untracked: false }).unwrap();
    assert_eq!(fs::read_to_string(p.join("file")).unwrap(), "second\n");
    fs::write(p.join("* literal"), "discard").unwrap();
    fs::write(p.join("keep"), "keep").unwrap();
    apply(p, Action::Discard { file: "* literal".into(), untracked: true }).unwrap();
    assert!(!p.join("* literal").exists());
    assert!(p.join("keep").exists());
    fs::remove_file(p.join("keep")).unwrap();
    apply(p, Action::CreateTag { name: "v1".into(), target: tip.clone(), message: "release".into() }).unwrap();
    let oid = git(p, &["rev-parse", "refs/tags/v1"]);
    apply(p, Action::DeleteTag { name: "v1".into(), expected: oid }).unwrap();
    assert!(advanced::resolve_target(p, "v1").is_err());
    apply(p, Action::BranchAt { name: "rescued".into(), target: base.clone() }).unwrap();
    apply(p, Action::DeleteBranch { branch: "rescued".into(), expected: base }).unwrap();
}
#[test]
fn interactive_rebase_and_squash_hook_failure_roll_back() {
    use std::os::unix::fs::PermissionsExt;
    let d = fixture();
    let p = d.path();
    let base = git(p, &["rev-parse", "HEAD"]);
    write_commit(p, "one", "one\n", "one");
    write_commit(p, "two", "two\n", "two");
    write_commit(p, "three", "three\n", "three");
    let mut steps = advanced::plan(p, &base).unwrap();
    steps.swap(0, 1);
    steps[1].action = Pick::Fixup;
    steps[2].action = Pick::Drop;
    apply(p, Action::Interactive { base: base.clone(), steps, editor: env!("CARGO_BIN_EXE_nyx-git").into() }).unwrap();
    assert_eq!(git(p, &["rev-list", "--count", &format!("{base}..HEAD")]), "1");
    assert!(p.join("one").exists());
    assert!(p.join("two").exists());
    assert!(!p.join("three").exists());
    write_commit(p, "three", "three\n", "three");
    let old = git(p, &["rev-parse", "HEAD"]);
    let hooks = p.join(".git/test-hooks");
    fs::create_dir(&hooks).unwrap();
    fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(hooks.join("pre-commit"), fs::Permissions::from_mode(0o755)).unwrap();
    git(p, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    assert!(apply(p, Action::Squash { base: base.clone(), message: "squashed".into() }).is_err());
    assert_eq!(git(p, &["rev-parse", "HEAD"]), old);
    assert!(git(p, &["status", "--porcelain"]).is_empty());
    git(p, &["config", "core.hooksPath", "/dev/null"]);
    apply(p, Action::Squash { base: base.clone(), message: "squashed".into() }).unwrap();
    assert_eq!(git(p, &["rev-list", "--count", &format!("{base}..HEAD")]), "1");
}
#[test]
fn exact_force_lease_rejects_remote_changes() {
    let d = fixture();
    let p = d.path();
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--bare"]);
    let url = remote.path().to_str().unwrap();
    git(p, &["remote", "add", "origin", url]);
    git(p, &["push", "origin", "main"]);
    let lease = advanced::remote_tip(p, "origin", "main").unwrap();
    write_commit(p, "file", "new\n", "new");
    git(p, &["push", "origin", "main"]);
    let remote_tip = git(p, &["rev-parse", "HEAD"]);
    apply(p, Action::Reset { target: lease.clone(), mode: ResetMode::Hard }).unwrap();
    assert!(apply(p, Action::ForcePush { remote: "origin".into(), branch: "main".into(), lease }).is_err());
    assert_eq!(git(remote.path(), &["rev-parse", "refs/heads/main"]), remote_tip);
    let lease = advanced::remote_tip(p, "origin", "main").unwrap();
    apply(p, Action::ForcePush { remote: "origin".into(), branch: "main".into(), lease }).unwrap();
    assert_eq!(git(remote.path(), &["rev-parse", "refs/heads/main"]), git(p, &["rev-parse", "HEAD"]));
}
