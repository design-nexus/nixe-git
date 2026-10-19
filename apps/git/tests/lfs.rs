use nixe_git::core::{
    lfs::{self, Action},
    operations::{self, Operation},
};
use std::{fs, path::Path, process::Command};
fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().into()
}
fn run(root: &Path, action: Action) -> anyhow::Result<Vec<u8>> {
    operations::run(root, Operation::Lfs { action, expected: Box::new(lfs::read(root)?) })
}
#[test]
fn native_tracking_fetch_and_stale_guards_preserve_index_and_contents() {
    assert!(
        Command::new("git").args(["lfs", "version"]).output().unwrap().status.success(),
        "Install git-lfs to run this integration test"
    );
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let remote = temp.path().join("remote.git");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare"]);
    git(&root, &["init", "-b", "main"]);
    for (key, value) in [("user.name", "LFS Test"), ("user.email", "lfs@example.invalid"), ("commit.gpgsign", "false")] {
        git(&root, &["config", key, value]);
    }
    git(&root, &["config", "core.hooksPath", root.join(".git/hooks").to_str().unwrap()]);
    git(&root, &["remote", "add", "origin", remote.to_str().unwrap()]);
    fs::write(root.join("asset 日本語.bin"), b"small offline LFS fixture\n").unwrap();
    run(&root, Action::Enable).unwrap();
    run(&root, Action::Track("*.bin".into())).unwrap();
    assert!(fs::read_to_string(root.join(".gitattributes")).unwrap().contains("*.bin filter=lfs"));
    assert!(git(&root, &["diff", "--cached", "--name-only"]).is_empty());
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "-m", "LFS asset"]);
    assert!(git(&root, &["show", "HEAD:asset 日本語.bin"]).starts_with("version https://git-lfs.github.com/spec/v1"));
    git(&root, &["push", "-u", "origin", "main"]);
    let head = git(&root, &["rev-parse", "HEAD"]);
    let index = fs::read(root.join(".git/index")).unwrap();
    let listing = lfs::read(&root).unwrap();
    assert!(listing.files.contains("asset 日本語.bin"));
    let objects = root.join(".git/lfs/objects");
    fs::rename(&objects, root.join(".git/lfs/saved-objects")).unwrap();
    run(&root, Action::Fetch("origin".into())).unwrap();
    assert!(objects.is_dir());
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(root.join("asset 日本語.bin")).unwrap(), b"small offline LFS fixture\n");
    let asset = root.join("asset 日本語.bin");
    let entries = git(&root, &["ls-files", "--stage"]);
    let pointer = git(&root, &["show", "HEAD:asset 日本語.bin"]);
    fs::write(&asset, format!("{pointer}\n")).unwrap();
    run(&root, Action::Checkout).unwrap();
    assert_eq!(fs::read(&asset).unwrap(), b"small offline LFS fixture\n");
    let before_edit = lfs::read(&root).unwrap();
    fs::write(&asset, "preserve my local edit\n").unwrap();
    assert!(operations::run(&root, Operation::Lfs { action: Action::Checkout, expected: Box::new(before_edit) }).is_err());
    run(&root, Action::Checkout).unwrap();
    assert_eq!(fs::read_to_string(&asset).unwrap(), "preserve my local edit\n");
    fs::remove_file(&asset).unwrap();
    run(&root, Action::Checkout).unwrap();
    assert_eq!(fs::read(&asset).unwrap(), b"small offline LFS fixture\n");
    assert_eq!(git(&root, &["ls-files", "--stage"]), entries);
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), head);
    let snapshot = lfs::read(&root).unwrap();
    fs::write(root.join(".gitattributes"), "*.bin filter=lfs diff=lfs merge=lfs -text\n# external edit\n").unwrap();
    assert!(
        operations::run(&root, Operation::Lfs { action: Action::Untrack("*.bin".into()), expected: Box::new(snapshot) }).is_err()
    );
    run(&root, Action::Untrack("*.bin".into())).unwrap();
    assert!(!fs::read_to_string(root.join(".gitattributes")).unwrap().contains("filter=lfs"));
    assert_eq!(git(&root, &["show", "HEAD:asset 日本語.bin"]).lines().next(), Some("version https://git-lfs.github.com/spec/v1"));
    assert!(run(&root, Action::Track("bad\npattern".into())).is_err());
    fs::remove_file(root.join(".gitattributes")).unwrap();
    let outside = temp.path().join("outside");
    fs::write(&outside, "preserve").unwrap();
    std::os::unix::fs::symlink(&outside, root.join(".gitattributes")).unwrap();
    assert!(lfs::read(&root).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "preserve");
}
