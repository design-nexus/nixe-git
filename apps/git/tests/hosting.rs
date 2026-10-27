use nyx_git::core::{
    advanced::Guard,
    hosting::{self, Checkout},
    operations::{self, Operation},
};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
fn git(path: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(path).args(args).output().unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().into()
}
#[test]
fn exact_request_checkout_uses_native_git_and_preserves_work_on_changed_requests() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("clone");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        git(&root, &["config", k, v]);
    }
    fs::write(root.join("file"), "base\n").unwrap();
    git(&root, &["add", "file"]);
    git(&root, &["commit", "-m", "base"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    git(&root, &["switch", "-c", "topic"]);
    fs::write(root.join("file"), "review\n").unwrap();
    git(&root, &["commit", "-am", "review"]);
    let expected = git(&root, &["rev-parse", "HEAD"]);
    git(&root, &["switch", "main"]);
    let remote = temp.path().join("remote.git");
    git(temp.path(), &["clone", "--bare", root.to_str().unwrap(), remote.to_str().unwrap()]);
    git(&remote, &["update-ref", "refs/pull/7/head", &expected]);
    let ssh = temp.path().join("fake-ssh");
    fs::write(&ssh, format!("#!/bin/sh\nexec git-upload-pack '{}'\n", remote.display())).unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    git(&root, &["config", "core.sshCommand", ssh.to_str().unwrap()]);
    git(&root, &["config", "ssh.variant", "ssh"]);
    git(&root, &["remote", "add", "origin", "git@provider.example:team/project.git"]);
    let request = |branch: &str, expected: String| Checkout {
        url: "git@provider.example:team/project.git".into(),
        reference: "refs/pull/7/head".into(),
        expected,
        branch: branch.into(),
        allowed_remotes: vec!["https://provider.example/team/project.git".into(), "git@provider.example:team/project.git".into()],
        guard: Box::new(Guard::capture(&root).unwrap()),
    };
    operations::run(&root, Operation::CheckoutRequest(request("review/request-7", expected.clone()))).unwrap();
    assert_eq!(git(&root, &["branch", "--show-current"]), "review/request-7");
    assert_eq!(fs::read_to_string(root.join("file")).unwrap(), "review\n");
    git(&root, &["switch", "main"]);
    git(&remote, &["update-ref", "refs/pull/7/head", &base]);
    assert!(operations::run(&root, Operation::CheckoutRequest(request("review/changed", expected.clone()))).is_err());
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), base);
    assert!(git(&root, &["status", "--porcelain"]).is_empty());
    let mut wrong = request("review/wrong", expected.clone());
    wrong.url = "git@elsewhere.example:team/project.git".into();
    wrong.allowed_remotes.push(wrong.url.clone());
    assert!(operations::run(&root, Operation::CheckoutRequest(wrong)).is_err());
    let stale = request("review/stale", expected);
    fs::write(root.join("file"), "local work\n").unwrap();
    assert!(operations::run(&root, Operation::CheckoutRequest(stale)).is_err());
    assert_eq!(fs::read_to_string(root.join("file")).unwrap(), "local work\n");
}
#[test]
fn clone_protocol_and_destination_guards_reject_unsafe_inputs() {
    for url in [
        "file:///tmp/repo",
        "ext::sh -c unsafe",
        "https://user:secret@example.com/repo.git",
        "https://example.com/repo.git?token=secret",
        "git://example.com/repo",
        "-option",
        "git@-oProxyCommand:repo",
    ] {
        assert!(hosting::transport(url).is_err(), "{url}");
    }
    assert_eq!(
        hosting::transport("git@example.com:team/project.git").unwrap(),
        hosting::transport("https://example.com/team/project.git").unwrap()
    );
    assert!(hosting::transport("ssh://git@example.com:2222/team/project.git").is_ok());
    assert_ne!(
        hosting::transport("https://example.com:8443/team/project.git").unwrap(),
        hosting::transport("https://example.com/team/project.git").unwrap()
    );
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("existing");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("precious"), "keep").unwrap();
    assert!(
        operations::run(
            temp.path(),
            Operation::Clone { url: "https://example.com/team/project.git".into(), destination: path.clone() }
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(path.join("precious")).unwrap(), "keep");
}
