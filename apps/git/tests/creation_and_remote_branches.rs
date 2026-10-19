use nixe_git::core::{
    advanced,
    operations::{self, Operation},
    remotes, repository,
};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn initialize(path: &Path, branch: &str) -> anyhow::Result<Vec<u8>> {
    operations::run(path, Operation::Initialize { destination: path.into(), branch: branch.into() })
}
fn commit(root: &Path, args: &[&str]) -> String {
    let mut all = vec![
        "-c",
        "user.name=Nixe Test",
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "core.hooksPath=/dev/null",
    ];
    all.extend_from_slice(args);
    git(root, &all)
}
#[test]
fn initializes_new_and_existing_directories_without_touching_files_or_reinitializing() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("existing files 日本語");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("keep"), "unchanged\n").unwrap();
    initialize(&root, "custom-start").unwrap();
    let snapshot = repository::load(&root).unwrap();
    assert!(snapshot.repository.unborn);
    assert_eq!(snapshot.repository.branch, "custom-start");
    assert_eq!(snapshot.status.files.len(), 1);
    assert_eq!(snapshot.status.files[0].index, '?');
    assert_eq!(fs::read_to_string(root.join("keep")).unwrap(), "unchanged\n");
    let head = fs::read(root.join(".git/HEAD")).unwrap();
    assert!(initialize(&root, "other").is_err());
    assert_eq!(fs::read(root.join(".git/HEAD")).unwrap(), head);
    assert!(!root.join(".git/index").exists());
    assert!(initialize(&root.join("nested"), "main").is_err());
    assert!(!root.join("nested").exists());
    let new = parent.path().join("new");
    initialize(&new, "main").unwrap();
    assert!(repository::load(&new).unwrap().repository.unborn);
    assert!(initialize(&parent.path().join("invalid"), "--bad").is_err());
    assert!(!parent.path().join("invalid").exists());
    let broken = parent.path().join("broken");
    fs::create_dir(&broken).unwrap();
    fs::write(broken.join(".git"), "broken metadata").unwrap();
    assert!(initialize(&broken, "main").is_err());
    assert_eq!(fs::read_to_string(broken.join(".git")).unwrap(), "broken metadata");
}
#[test]
fn deletes_only_push_destination_with_recovery_default_protection_and_exact_tip_race_rejection() {
    use std::os::unix::fs::PermissionsExt;
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("client");
    initialize(&root, "main").unwrap();
    commit(&root, &["commit", "--allow-empty", "-m", "Root"]);
    git(&root, &["config", "core.hooksPath", "/dev/null"]);
    let head = git(&root, &["rev-parse", "HEAD"]);
    let fetch = parent.path().join("fetch.git");
    let push = parent.path().join("push.git");
    for remote in [&fetch, &push] {
        fs::create_dir(remote).unwrap();
        git(remote, &["init", "--bare", "-b", "main"]);
        git(remote, &["config", "core.hooksPath", "/dev/null"]);
        git(&root, &["push", remote.to_str().unwrap(), "main"]);
        git(remote, &["update-ref", "refs/heads/delete-me", &head]);
    }
    git(&root, &["remote", "add", "origin", fetch.to_str().unwrap()]);
    git(&root, &["remote", "set-url", "--push", "origin", push.to_str().unwrap()]);
    let old = commit(&push, &["commit-tree", "refs/heads/main^{tree}", "-p", &head, "-m", "Server-only tip"]);
    let new = commit(&push, &["commit-tree", "refs/heads/main^{tree}", "-p", &old, "-m", "Concurrent server change"]);
    git(&push, &["update-ref", "refs/heads/delete-me", &old]);
    fs::write(root.join("keep"), "uncommitted\n").unwrap();
    let hooks = root.join("hooks");
    fs::create_dir(&hooks).unwrap();
    let quoted = push.to_str().unwrap().replace('\'', "'\\''");
    fs::write(
        hooks.join("pre-push"),
        format!("#!/bin/sh\ngit --git-dir='{quoted}' update-ref refs/heads/delete-me {new} {old}\n"),
    )
    .unwrap();
    fs::set_permissions(hooks.join("pre-push"), fs::Permissions::from_mode(0o755)).unwrap();
    git(&root, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let listing = remotes::branches(&root, "origin").unwrap();
    assert_eq!(listing.default.as_deref(), Some("main"));
    assert_eq!(listing.branches.iter().find(|b| b.name == "delete-me").unwrap().id, old);
    let delete = |listing, branch: &str, expected: &str| {
        operations::run(
            &root,
            Operation::DeleteRemoteBranch { listing: Box::new(listing), branch: branch.into(), expected: expected.into() },
        )
    };
    assert!(delete(listing.clone(), "main", &head).is_err());
    assert!(delete(listing, "delete-me", &old).is_err());
    assert_eq!(git(&push, &["rev-parse", "refs/heads/delete-me"]), new);
    assert!(advanced::recovery(&root).unwrap().iter().any(|r| r.id == old && r.label.starts_with("refs/nixe/recovery/")));
    fs::remove_file(hooks.join("pre-push")).unwrap();
    let listing = remotes::branches(&root, "origin").unwrap();
    delete(listing, "delete-me", &new).unwrap();
    assert!(git(&push, &["for-each-ref", "refs/heads/delete-me"]).is_empty());
    assert_eq!(git(&fetch, &["rev-parse", "refs/heads/delete-me"]), head);
    assert!(advanced::recovery(&root).unwrap().iter().any(|r| r.id == new && r.label.starts_with("refs/nixe/recovery/")));
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read_to_string(root.join("keep")).unwrap(), "uncommitted\n");
}
