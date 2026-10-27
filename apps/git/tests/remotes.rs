use nyx_git::core::{
    advanced,
    operations::{self, Operation},
    remotes::{self, Action},
};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-b", "main"]);
    git(
        dir.path(),
        &[
            "-c",
            "user.name=Remote Test",
            "-c",
            "user.email=remote@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--allow-empty",
            "-m",
            "Root",
        ],
    );
    fs::write(dir.path().join("keep"), "working contents\n").unwrap();
    dir
}
fn change(root: &Path, action: Action) -> anyhow::Result<Vec<u8>> {
    let expected = Box::new(remotes::read(root)?);
    operations::run(root, Operation::Remote { action, expected })
}
#[test]
fn native_remote_edits_preserve_work_and_retain_removed_tracking_tips() {
    let dir = fixture();
    let root = dir.path();
    let head = git(root, &["rev-parse", "HEAD"]);
    let literal = root.join("remote with spaces $(literal).git").to_string_lossy().to_string();
    change(root, Action::Add { name: "origin".into(), url: literal.clone() }).unwrap();
    assert_eq!(remotes::read(root).unwrap().remotes[0].fetch, vec![literal]);
    change(root, Action::FetchUrl { name: "origin".into(), url: "git@example.invalid:owner/repo.git".into() }).unwrap();
    change(root, Action::PushUrl { name: "origin".into(), url: Some("ssh://git@example.invalid/owner/write.git".into()) })
        .unwrap();
    assert_eq!(git(root, &["remote", "get-url", "--push", "origin"]), "ssh://git@example.invalid/owner/write.git");
    change(root, Action::PushUrl { name: "origin".into(), url: None }).unwrap();
    assert!(remotes::read(root).unwrap().remotes[0].push.is_empty());
    let tip = git(
        root,
        &[
            "-c",
            "user.name=Remote Test",
            "-c",
            "user.email=remote@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit-tree",
            "HEAD^{tree}",
            "-p",
            &head,
            "-m",
            "Remote-only commit",
        ],
    );
    git(root, &["update-ref", "refs/remotes/origin/main", &tip]);
    git(root, &["config", "branch.main.remote", "origin"]);
    git(root, &["config", "branch.main.merge", "refs/heads/main"]);
    change(root, Action::Rename { name: "origin".into(), new: "upstream".into() }).unwrap();
    assert_eq!(git(root, &["config", "branch.main.remote"]), "upstream");
    assert_eq!(git(root, &["rev-parse", "refs/remotes/upstream/main"]), tip);
    change(root, Action::Remove { name: "upstream".into() }).unwrap();
    assert!(remotes::read(root).unwrap().remotes.is_empty());
    assert!(git(root, &["for-each-ref", "refs/remotes/"]).is_empty());
    assert!(advanced::recovery(root).unwrap().iter().any(|r| r.id == tip && r.label.starts_with("refs/nyx/recovery/")));
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read_to_string(root.join("keep")).unwrap(), "working contents\n");
}
#[test]
fn stale_multivalue_included_and_unsafe_changes_are_refused() {
    let dir = fixture();
    let root = dir.path();
    change(root, Action::Add { name: "origin".into(), url: "https://example.invalid/repo.git".into() }).unwrap();
    let before = fs::read(root.join(".git/config")).unwrap();
    for action in [
        Action::Add { name: "--bad".into(), url: "https://example.invalid/x".into() },
        Action::FetchUrl { name: "origin".into(), url: "https://user:secret@example.invalid/x".into() },
        Action::PushUrl { name: "origin".into(), url: Some("ext::dangerous".into()) },
    ] {
        assert!(change(root, action).is_err());
    }
    assert_eq!(fs::read(root.join(".git/config")).unwrap(), before);
    assert!(!remotes::display_url("https://user:secret@example.invalid/x").contains("secret"));
    let expected = Box::new(remotes::read(root).unwrap());
    git(root, &["config", "remote.origin.url", "https://example.invalid/changed.git"]);
    assert!(operations::run(root, Operation::Remote { action: Action::Remove { name: "origin".into() }, expected }).is_err());
    git(root, &["config", "--add", "remote.origin.url", "https://example.invalid/second.git"]);
    assert!(change(root, Action::FetchUrl { name: "origin".into(), url: "https://example.invalid/new.git".into() }).is_err());
    git(root, &["config", "remote.origin.fetch", "+refs/*:refs/custom/*"]);
    assert!(change(root, Action::Remove { name: "origin".into() }).is_err());
    let include = root.join("included-config");
    fs::write(
        &include,
        "[remote \"inherited\"]\nurl = https://example.invalid/inherited.git\nfetch = +refs/heads/*:refs/remotes/inherited/*\n",
    )
    .unwrap();
    git(root, &["config", "include.path", include.to_str().unwrap()]);
    let snapshot = remotes::read(root).unwrap();
    assert!(!snapshot.remotes.iter().find(|r| r.name == "inherited").unwrap().editable);
    assert!(change(root, Action::Rename { name: "inherited".into(), new: "new".into() }).is_err());
}
