use nixe_git::core::{
    advanced::Guard,
    workspace::Workspace,
    worktrees::{self, Action, State},
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
fn fixture() -> (tempfile::TempDir, PathBuf, String) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        git(&root, &["config", key, value]);
    }
    fs::write(root.join("file"), "initial\n").unwrap();
    fs::write(root.join(".gitignore"), "ignored\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "-m", "base"]);
    let head = git(&root, &["rev-parse", "HEAD"]);
    (temp, root, head)
}
fn create(root: &Path, path: PathBuf, branch: &str, new_branch: bool, base: &str) {
    worktrees::run(root, Action::Create { path, branch: branch.into(), new_branch, base: base.into() }).unwrap();
}
#[test]
fn create_existing_and_new_branches_lock_and_remove_safely() {
    let (temp, root, head) = fixture();
    let linked = temp.path().join("feature space 日本語");
    create(&root, linked.clone(), "feature", true, &head);
    assert_eq!(git(&linked, &["branch", "--show-current"]), "feature");
    assert!(
        worktrees::run(
            &root,
            Action::Create {
                path: temp.path().join("duplicate"),
                branch: "feature".into(),
                new_branch: false,
                base: head.clone()
            }
        )
        .is_err()
    );
    git(&root, &["branch", "existing"]);
    let existing = temp.path().join("existing");
    create(&root, existing.clone(), "existing", false, &head);
    assert_eq!(git(&existing, &["branch", "--show-current"]), "existing");
    let w = worktrees::list(&root).unwrap().into_iter().find(|w| w.path == linked).unwrap();
    worktrees::run(&root, Action::Lock { expected: w, reason: "external disk".into() }).unwrap();
    let w = worktrees::list(&root).unwrap().into_iter().find(|w| w.path == linked).unwrap();
    assert_eq!(w.locked.as_deref(), Some("external disk"));
    assert!(worktrees::removal_guard(&root, &w).is_err());
    worktrees::run(&root, Action::Unlock { expected: w }).unwrap();
    let w = worktrees::list(&root).unwrap().into_iter().find(|w| w.path == linked).unwrap();
    let guard = worktrees::removal_guard(&root, &w).unwrap();
    worktrees::run(&root, Action::Remove { expected: w, guard: Box::new(guard) }).unwrap();
    assert!(!linked.exists());
    assert_eq!(git(&root, &["rev-parse", "feature"]), head);
    let main = worktrees::list(&root).unwrap().into_iter().find(|w| w.main).unwrap();
    assert!(worktrees::removal_guard(&root, &main).is_err());
    let active = worktrees::list(&existing).unwrap().into_iter().find(|w| w.path == existing).unwrap();
    assert!(worktrees::removal_guard(&existing, &active).is_err());
    git(&existing, &["checkout", "--detach"]);
    git(&existing, &["commit", "--allow-empty", "-m", "detached work"]);
    let detached = git(&existing, &["rev-parse", "HEAD"]);
    let w = worktrees::list(&root).unwrap().into_iter().find(|w| w.path == existing).unwrap();
    let guard = worktrees::removal_guard(&root, &w).unwrap();
    worktrees::run(&root, Action::Remove { expected: w, guard: Box::new(guard) }).unwrap();
    assert!(nixe_git::core::advanced::recovery(&root).unwrap().iter().any(|r| r.id == detached));
}
#[test]
fn dirty_ignored_stale_guard_and_branch_move_are_rejected() {
    let (temp, root, head) = fixture();
    let linked = temp.path().join("linked");
    create(&root, linked.clone(), "feature", true, &head);
    let w = worktrees::list(&root).unwrap().into_iter().find(|w| w.path == linked).unwrap();
    let guard = worktrees::removal_guard(&root, &w).unwrap();
    fs::write(linked.join("file"), "changed\n").unwrap();
    assert!(worktrees::run(&root, Action::Remove { expected: w.clone(), guard: Box::new(guard) }).is_err());
    assert!(linked.exists());
    assert!(matches!(
        worktrees::overview(&root).unwrap().into_iter().find(|o| o.worktree.path == linked).unwrap().state,
        State::Dirty(_)
    ));
    git(&linked, &["restore", "file"]);
    fs::write(linked.join("ignored"), "precious cache\n").unwrap();
    assert!(matches!(
        worktrees::overview(&root).unwrap().into_iter().find(|o| o.worktree.path == linked).unwrap().state,
        State::Ignored
    ));
    assert!(worktrees::removal_guard(&root, &w).is_err());
    // Even a caller supplying an ordinary guard cannot bypass ignored-file protection.
    assert!(worktrees::run(&root, Action::Remove { expected: w, guard: Box::new(Guard::capture(&linked).unwrap()) }).is_err());
    assert_eq!(fs::read_to_string(linked.join("ignored")).unwrap(), "precious cache\n");
    git(&root, &["branch", "moving"]);
    git(&root, &["commit", "--allow-empty", "-m", "advance"]);
    git(&root, &["branch", "-f", "moving", "HEAD"]);
    assert!(
        worktrees::run(
            &root,
            Action::Create { path: temp.path().join("moved"), branch: "moving".into(), new_branch: false, base: head.clone() }
        )
        .is_err()
    );
    assert!(
        worktrees::run(
            &root,
            Action::Create { path: root.join("nested"), branch: "nested".into(), new_branch: true, base: head }
        )
        .is_err()
    );
}
#[test]
fn prune_missing_worktrees_keeps_locked_registrations_and_rejects_stale_preview() {
    let (temp, root, head) = fixture();
    let missing = temp.path().join("missing");
    let locked = temp.path().join("locked");
    create(&root, missing.clone(), "missing", true, &head);
    create(&root, locked.clone(), "locked", true, &head);
    let w = worktrees::list(&root).unwrap().into_iter().find(|w| w.path == locked).unwrap();
    worktrees::run(&root, Action::Lock { expected: w, reason: "offline".into() }).unwrap();
    fs::remove_dir_all(&missing).unwrap();
    fs::remove_dir_all(&locked).unwrap();
    let inventory = worktrees::list(&root).unwrap();
    assert!(inventory.iter().find(|w| w.path == missing).unwrap().prunable.is_some());
    git(&root, &["commit", "--allow-empty", "-m", "new head"]);
    assert!(worktrees::run(&root, Action::Prune { expected: inventory }).is_err());
    let expected = worktrees::list(&root).unwrap();
    worktrees::run(&root, Action::Prune { expected }).unwrap();
    let after = worktrees::list(&root).unwrap();
    assert!(!after.iter().any(|w| w.path == missing));
    assert!(after.iter().any(|w| w.path == locked && w.locked.is_some()));
    assert_eq!(git(&root, &["rev-parse", "missing"]), head);
}
#[test]
fn nul_paths_and_workspace_session_roundtrip() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let mut raw = b"worktree /tmp/raw\xff\npath\0HEAD ".to_vec();
    raw.extend_from_slice("a".repeat(40).as_bytes());
    raw.extend_from_slice(b"\0branch refs/heads/topic\0locked reason\nwith newline\0\0");
    let records = worktrees::parse(&raw).unwrap();
    assert_eq!(records[0].path, PathBuf::from(OsString::from_vec(b"/tmp/raw\xff\npath".to_vec())));
    assert_eq!(records[0].locked.as_deref(), Some("reason\nwith newline"));
    let mut w = Workspace::default();
    let a = PathBuf::from("/tmp/repo a");
    let b = PathBuf::from("/tmp/repo b");
    w.activate(a.clone());
    w.activate(b.clone());
    w.activate(a.clone());
    assert_eq!(w.open.len(), 2);
    assert_eq!(w.next(false), Some(b.clone()));
    assert_eq!(w.next(true), Some(b.clone()));
    let e = w.entry(&a);
    e.favorite = true;
    e.group = "Nixe".into();
    e.message = "unfinished commit\nbody".into();
    e.details_position = 700;
    e.narrow_position = 350;
    e.zoom = 1.3;
    let mut restored: Workspace = serde_json::from_slice(&serde_json::to_vec(&w).unwrap()).unwrap();
    assert_eq!(restored.open, w.open);
    assert_eq!(restored.active, Some(a.clone()));
    assert_eq!(restored.entry(&a).message, "unfinished commit\nbody");
    assert!(restored.entry(&a).favorite);
    assert_eq!(restored.entry(&a).zoom, 1.3);
    assert_eq!(restored.close(&a), Some(b.clone()));
    assert_eq!(restored.close(&b), None);
    assert!(restored.open.is_empty());
    assert_eq!(restored.entry(&a).message, "unfinished commit\nbody");
}
