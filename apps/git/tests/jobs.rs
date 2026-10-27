use nyx_git::core::{
    jobs::{self, Control},
    operations::{self, Operation},
    repository, watch,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().into()
}
fn fixture(parent: &Path, name: &str) -> PathBuf {
    let p = parent.join(name);
    fs::create_dir(&p).unwrap();
    git(&p, &["init", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        git(&p, &["config", k, v]);
    }
    fs::write(p.join("file"), "original\n").unwrap();
    git(&p, &["add", "file"]);
    git(&p, &["commit", "-m", "base"]);
    p
}
#[test]
fn cancellation_stops_git_and_ssh_children_without_touching_local_work() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(temp.path(), "repo");
    let ready = temp.path().join("ready");
    let pidfile = temp.path().join("pid");
    let ssh = temp.path().join("ssh-helper");
    fs::write(
        &ssh,
        format!(
            "#!/bin/sh\necho $$ > '{}'\necho ready > '{}'\necho 'Receiving objects: 10% (1/10)' >&2\nexec sleep 30\n",
            pidfile.display(),
            ready.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    git(&root, &["config", "core.sshCommand", ssh.to_str().unwrap()]);
    git(&root, &["config", "ssh.variant", "ssh"]);
    git(&root, &["remote", "add", "origin", "git@example.invalid:team/repo.git"]);
    fs::write(root.join("file"), "local changes\n").unwrap();
    let head = git(&root, &["rev-parse", "HEAD"]);
    let index = fs::read(root.join(".git/index")).unwrap();
    let control = Control::new(true);
    let c = control.clone();
    let p = root.clone();
    let worker = thread::spawn(move || jobs::controlled(c, || operations::run(&p, Operation::Fetch)));
    let start = Instant::now();
    while !ready.exists() && start.elapsed() < Duration::from_secs(3) {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists());
    assert!(control.cancel());
    let result = worker.join().unwrap();
    assert!(result.unwrap_err().to_string().contains("Stopped on request"));
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read_to_string(root.join("file")).unwrap(), "local changes\n");
    let pid = fs::read_to_string(pidfile).unwrap();
    let running = fs::read_to_string(format!("/proc/{}/stat", pid.trim()))
        .is_ok_and(|s| s.split(')').nth(1).is_some_and(|s| !s.trim_start().starts_with('Z')));
    assert!(!running, "The SSH helper survived cancellation");
    assert!(!Control::new(false).cancel());
}
#[test]
fn common_worktree_locks_queue_fetches_and_allow_independent_repositories() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(temp.path(), "repo");
    let other = fixture(temp.path(), "other");
    let linked = temp.path().join("linked");
    git(&root, &["worktree", "add", "-b", "topic", linked.to_str().unwrap()]);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let r = root.clone();
    let worker = thread::spawn(move || {
        jobs::locked(&r, || {
            jobs::locked(&r, || Ok(()))?;
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
    });
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(jobs::locked(&linked, || Ok(())).is_err());
    jobs::locked(&other, || Ok(())).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_child_probe", "--ignored"])
        .env("NYX_GIT_LOCK_PROBE", &root)
        .output()
        .unwrap();
    assert!(child.status.success(), "{}", String::from_utf8_lossy(&child.stdout));
    let control = Control::new(true);
    let c = control.clone();
    let p = linked.clone();
    let queued = thread::spawn(move || jobs::controlled(c, || jobs::locked(&p, || Ok(()))));
    let end = Instant::now() + Duration::from_secs(2);
    while !control.progress().contains("Waiting") && Instant::now() < end {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(control.progress().contains("Waiting"));
    control.cancel();
    assert!(queued.join().unwrap().is_err());
    release_tx.send(()).unwrap();
    worker.join().unwrap().unwrap();
    jobs::locked(&linked, || Ok(())).unwrap();
}
#[test]
#[ignore = "Spawned by the cross-process lock test"]
fn lock_child_probe() {
    let root = PathBuf::from(std::env::var_os("NYX_GIT_LOCK_PROBE").unwrap());
    assert!(jobs::locked(&root, || Ok(())).is_err());
}
#[test]
fn watch_plans_cover_nested_paths_and_refs_without_walking_ignored_caches() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(temp.path(), "repo");
    fs::create_dir_all(root.join("src/nested")).unwrap();
    fs::write(root.join("src/nested/tracked"), "tracked\n").unwrap();
    fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    fs::create_dir_all(root.join("ignored/deep/cache")).unwrap();
    fs::write(root.join("ignored/deep/cache/file"), "cache\n").unwrap();
    git(&root, &["add", "src", ".gitignore"]);
    git(&root, &["commit", "-m", "nested"]);
    git(&root, &["branch", "group/topic"]);
    let repo = repository::inspect(&root).unwrap();
    let plan = watch::plan(&repo).unwrap();
    assert!(plan.directories.contains(&root.join("src/nested")));
    assert!(plan.directories.contains(&root.join(".git/refs/heads/group")));
    assert!(!plan.directories.iter().any(|p| p.starts_with(root.join("ignored"))));
    assert!(plan.directories.len() <= 640);
    let linked = temp.path().join("linked");
    git(&root, &["worktree", "add", "-b", "work", linked.to_str().unwrap()]);
    let repo = repository::inspect(&linked).unwrap();
    let plan = watch::plan(&repo).unwrap();
    assert!(plan.directories.contains(&repo.git_dir));
    assert!(plan.directories.contains(&repo.common_dir));
    assert!(plan.directories.contains(&linked.join("src/nested")));
}
