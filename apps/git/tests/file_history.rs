use nixe_git::core::{
    file_history,
    jobs::{self, Control},
};
use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::ffi::OsStringExt,
    path::Path,
    process::Command,
};

fn git(root: &Path, args: &[&OsStr]) -> String {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn run(root: &Path, args: &[&str]) -> String {
    git(root, &args.iter().map(OsStr::new).collect::<Vec<_>>())
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    run(dir.path(), &["init", "-b", "main"]);
    for (key, value) in [
        ("user.name", "File History Test"),
        ("user.email", "history@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        run(dir.path(), &["config", key, value]);
    }
    dir
}
fn commit(root: &Path, message: &str) {
    run(root, &["add", "--all"]);
    run(root, &["commit", "-m", message]);
}
#[test]
fn pinned_pages_follow_renames_with_literal_non_utf8_paths_and_preserve_local_state() {
    let dir = fixture();
    let root = dir.path();
    let old = OsString::from_vec(b"old * raw\xff\nfile".to_vec());
    let new = OsString::from_vec(b"new * raw\xff\nfile".to_vec());
    fs::write(root.join(&old), "original\n").unwrap();
    fs::write(root.join("other"), "other-only\n").unwrap();
    commit(root, "Create both files");
    fs::rename(root.join(&old), root.join(&new)).unwrap();
    commit(root, "Rename file");
    fs::write(root.join(&new), "updated\n").unwrap();
    commit(root, "Edit file");
    let first = file_history::page(root, Path::new(&new), None, 0, 1).unwrap();
    assert!(first.older);
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.entries[0].message.trim(), "Edit file");
    assert!(String::from_utf8(file_history::diff(root, &first.entries[0]).unwrap()).unwrap().contains("+updated"));
    fs::write(root.join(&new), "later commit\n").unwrap();
    commit(root, "Later HEAD");
    let expected_head = run(root, &["rev-parse", "HEAD"]);
    fs::write(root.join(&new), "uncommitted\n").unwrap();
    let index = fs::read(root.join(".git/index")).unwrap();
    let second = file_history::page(root, Path::new(&new), first.anchor.as_deref(), 1, 1).unwrap();
    assert!(second.older);
    assert_eq!(second.entries[0].path, Path::new(&new));
    assert_eq!(second.entries[0].previous_path.as_deref(), Some(Path::new(&old)));
    let rename = String::from_utf8(file_history::diff(root, &second.entries[0]).unwrap()).unwrap();
    assert!(rename.contains("rename from"), "{rename}");
    let third = file_history::page(root, Path::new(&new), first.anchor.as_deref(), 2, 1).unwrap();
    assert!(!third.older);
    assert_eq!(third.entries[0].path, Path::new(&old));
    let creation = String::from_utf8(file_history::diff(root, &third.entries[0]).unwrap()).unwrap();
    assert!(creation.contains("+original"));
    assert!(!creation.contains("other-only"));
    assert_eq!(run(root, &["rev-parse", "HEAD"]), expected_head);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read_to_string(root.join(&new)).unwrap(), "uncommitted\n");
}
#[test]
fn empty_deleted_and_shallow_histories_and_cancellation_are_explicit() {
    let dir = fixture();
    let root = dir.path();
    assert!(file_history::page(root, Path::new("file"), None, 0, 2).unwrap().entries.is_empty());
    assert!(file_history::page(root, Path::new("../outside"), None, 0, 2).is_err());
    assert!(file_history::page(root, Path::new("file"), None, 0, 51).is_err());
    fs::write(root.join("file"), "original\n").unwrap();
    commit(root, "Create");
    fs::write(root.join("file"), "modified\n").unwrap();
    commit(root, "Modify");
    let clone_parent = tempfile::tempdir().unwrap();
    let shallow = clone_parent.path().join("shallow");
    run(clone_parent.path(), &["clone", "--depth=1", &format!("file://{}", root.display()), shallow.to_str().unwrap()]);
    let page = file_history::page(&shallow, Path::new("file"), None, 0, 2).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert!(file_history::diff(&shallow, &page.entries[0]).unwrap_err().to_string().contains("Shallow"));
    fs::remove_file(root.join("file")).unwrap();
    commit(root, "Delete");
    let page = file_history::page(root, Path::new("file"), None, 0, 2).unwrap();
    assert_eq!(page.entries[0].message.trim(), "Delete");
    assert!(String::from_utf8(file_history::diff(root, &page.entries[0]).unwrap()).unwrap().contains("-modified"));
    let control = Control::new(true);
    control.cancel();
    assert!(jobs::controlled(control, || file_history::page(root, Path::new("file"), None, 0, 2)).is_err());
}
