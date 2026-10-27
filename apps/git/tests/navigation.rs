use nyx_git::core::{
    navigation::{self, Action, Kind},
    workspace::Workspace,
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().into()
}
#[test]
fn bounded_sidebar_inventory_reads_refs_stashes_worktrees_and_remotes_without_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Sidebar Test"),
        ("user.email", "sidebar@example.invalid"),
        ("commit.gpgsign", "false"),
        ("tag.gpgSign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        git(&root, &["config", key, value]);
    }
    fs::write(root.join("file"), "original\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "-m", "Initial"]);
    let head = git(&root, &["rev-parse", "HEAD"]);
    git(&root, &["branch", "feature/日本語"]);
    git(&root, &["tag", "light"]);
    git(&root, &["tag", "-a", "annotated", "-m", "Tag"]);
    let blob = git(&root, &["hash-object", "-w", "file"]);
    git(&root, &["update-ref", "refs/tags/blob", &blob]);
    git(&root, &["remote", "add", "origin", "https://example.invalid/repo.git"]);
    git(&root, &["update-ref", "refs/remotes/origin/main", &head]);
    git(&root, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    git(&root, &["config", "branch.main.remote", "origin"]);
    git(&root, &["config", "branch.main.merge", "refs/heads/main"]);
    fs::write(root.join("file"), "stash contents\n").unwrap();
    git(&root, &["stash", "push", "-m", "Saved sidebar work"]);
    let linked = temp.path().join("linked 日本語");
    git(&root, &["worktree", "add", "-b", "parallel", linked.to_str().unwrap()]);
    fs::write(root.join("file"), "uncommitted\n").unwrap();
    let index = fs::read(root.join(".git/index")).unwrap();
    let inventory = navigation::read(&root).unwrap();
    let watch = nyx_git::core::watch::plan(&nyx_git::core::repository::inspect(&root).unwrap()).unwrap();
    assert!(watch.directories.contains(&root.join(".git/worktrees")));
    let group = |kind| inventory.groups.iter().find(|g| g.kind == kind).unwrap();
    assert!(group(Kind::Local).items.iter().any(|i| i.name == "main"
        && i.detail.contains("Tracks origin/main")
        && matches!(i.action, Action::Reference { current: true, .. })));
    assert!(group(Kind::Local).items.iter().any(|i| i.name == "feature/日本語"));
    assert_eq!(group(Kind::RemoteBranch).items.len(), 1);
    assert!(
        group(Kind::Tag)
            .items
            .iter()
            .any(|i| i.name == "annotated" && matches!(i.action, Action::Reference { commit: true, .. }))
    );
    assert!(
        group(Kind::Tag).items.iter().any(|i| i.name == "blob" && matches!(i.action, Action::Reference { commit: false, .. }))
    );
    assert!(group(Kind::Stash).items[0].detail.contains("Saved sidebar work"));
    assert!(group(Kind::Worktree).items.iter().any(|i| matches!(&i.action,Action::Worktree(path)if path==&linked)));
    assert_eq!(group(Kind::Remote).items[0].name, "origin");
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read_to_string(root.join("file")).unwrap(), "uncommitted\n");
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["update-ref", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let input = (0..=navigation::LIMIT).map(|i| format!("create refs/tags/many/{i:04} {head}\n")).collect::<String>();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
    let limited = navigation::read(&root).unwrap();
    let tags = limited.groups.iter().find(|g| g.kind == Kind::Tag).unwrap();
    assert_eq!(tags.items.len(), navigation::LIMIT);
    assert!(tags.limited);
    let mut workspace: Workspace = serde_json::from_str("{}").unwrap();
    workspace.entry(&root).sidebar_closed = vec!["tags".into()];
    workspace.entry(&linked).sidebar_closed = vec!["stashes".into()];
    let mut restored: Workspace = serde_json::from_str(&serde_json::to_string(&workspace).unwrap()).unwrap();
    assert_eq!(restored.entry(&root).sidebar_closed, vec!["tags"]);
    assert_eq!(restored.entry(&linked).sidebar_closed, vec!["stashes"]);
}
