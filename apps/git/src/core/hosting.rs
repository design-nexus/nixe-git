//! Git transport for optional hosting workflows, through the shared process boundary.
use super::{advanced::Guard, operations};
use anyhow::{Context, Result, ensure};
use std::{ffi::OsString, path::Path};
use url::Url;
pub fn transport(value: &str) -> Result<(String, String)> {
    ensure!(!value.is_empty() && !value.chars().any(char::is_control), "Invalid clone URL");
    if let Ok(url) = Url::parse(value) {
        ensure!(matches!(url.scheme(), "https" | "ssh") && url.host_str().is_some(), "Use an HTTPS or SSH clone URL");
        ensure!(
            url.password().is_none()
                && (url.scheme() == "ssh" || url.username().is_empty())
                && url.query().is_none()
                && url.fragment().is_none(),
            "Clone URLs must not contain passwords, HTTPS usernames, queries or fragments"
        );
        ensure!(
            url.scheme() != "ssh" || url.username().chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)),
            "Invalid SSH username"
        );
        ensure!(!url.host_str().unwrap().starts_with('-'), "Invalid SSH host");
        let path = url.path().trim_matches('/').trim_end_matches(".git");
        ensure!(!path.is_empty(), "Missing repository path");
        let host = url.host_str().unwrap().to_lowercase();
        let host = match url.port() {
            Some(port) if !(url.scheme() == "ssh" && port == 22) => format!("{host}:{port}"),
            _ => host,
        };
        return Ok((host, path.into()));
    }
    let (host, path) = value.split_once(':').context("Use an HTTPS or SSH clone URL")?;
    let (user, host) = host.split_once('@').context("Use a complete SSH clone URL")?;
    ensure!(!user.is_empty() && user.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)), "Invalid SSH user");
    ensure!(
        !host.is_empty() && !host.starts_with('-') && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c)),
        "Invalid SSH host"
    );
    let path = path.trim_matches('/').trim_end_matches(".git");
    ensure!(!path.is_empty() && !path.chars().any(char::is_whitespace), "Invalid repository path");
    Ok((host.to_lowercase(), path.into()))
}
pub fn clone_repository(parent: &Path, url: &str, destination: &Path) -> Result<Vec<u8>> {
    transport(url)?;
    ensure!(destination.is_absolute(), "Choose an absolute clone directory");
    ensure!(
        matches!(std::fs::symlink_metadata(destination),Err(e)if e.kind()==std::io::ErrorKind::NotFound),
        "The clone destination already exists or cannot be inspected"
    );
    let real_parent = destination.parent().context("Missing parent directory")?.canonicalize()?;
    ensure!(parent.canonicalize()? == real_parent, "The clone destination parent changed");
    let a: Vec<OsString> = ["-c", "protocol.ext.allow=never", "-c", "protocol.file.allow=never", "clone", "--progress", "--"]
        .iter()
        .map(OsString::from)
        .chain([url.into(), destination.as_os_str().into()])
        .collect();
    operations::invoke(parent, "Clone repository", a, None, &[], false)
}
#[derive(Clone, Debug)]
pub struct Checkout {
    pub url: String,
    pub reference: String,
    pub expected: String,
    pub branch: String,
    pub allowed_remotes: Vec<String>,
    pub guard: Box<Guard>,
}
pub fn checkout(root: &Path, request: Checkout) -> Result<Vec<u8>> {
    request.guard.verify(root)?;
    request.guard.clean()?;
    ensure!(request.guard.progress.is_none(), "Finish the active Git operation first");
    super::advanced::valid_ref("refs/heads/", &request.branch)?;
    operations::valid_id(&request.expected)?;
    let expected = transport(&request.url)?;
    ensure!(
        request.allowed_remotes.iter().filter_map(|u| transport(u).ok()).any(|id| id == expected),
        "The request URL does not match its repository"
    );
    let bytes = operations::invoke(
        root,
        "Read configured remotes",
        vec!["config".into(), "--null".into(), "--get-regexp".into(), "^remote\\..*\\.url$".into()],
        None,
        &[],
        true,
    )?;
    let matched = bytes
        .split(|b| *b == 0)
        .filter_map(|r| r.splitn(2, |b| *b == b'\n').nth(1))
        .filter_map(|r| std::str::from_utf8(r).ok())
        .filter_map(|r| transport(r).ok())
        .any(|id| id == expected);
    ensure!(
        matched,
        "The active repository has no remote matching this hosted repository. Open its clone before checking out requests."
    );
    let pieces = request.reference.split('/').collect::<Vec<_>>();
    ensure!(
        pieces.len() == 4
            && pieces[0] == "refs"
            && matches!(pieces[1], "pull" | "merge-requests")
            && pieces[2].parse::<u64>().is_ok_and(|n| n > 0)
            && pieces[3] == "head",
        "Invalid request ref"
    );
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let local = format!("refs/nyx/requests/{stamp}-{}", std::process::id());
    let args: Vec<OsString> = [
        "-c",
        "protocol.ext.allow=never",
        "-c",
        "protocol.file.allow=never",
        "fetch",
        "--no-tags",
        "--no-recurse-submodules",
        "--",
    ]
    .iter()
    .map(OsString::from)
    .chain([request.url.into(), format!("{}:{local}", request.reference).into()])
    .collect();
    operations::invoke(root, "Fetch request branch", args, None, &[], false)?;
    let repo = gix::discover(root)?;
    let actual = repo.find_reference(local.as_str())?.peel_to_commit()?.id.to_string();
    ensure!(
        actual == request.expected,
        "The request branch changed while fetching. Refresh its details and retry; the fetched commit remains in its private Nyx ref."
    );
    request.guard.verify(root)?;
    operations::invoke(
        root,
        "Checkout request branch",
        vec!["switch".into(), "--no-guess".into(), "--create".into(), request.branch.into(), actual.into()],
        None,
        &[],
        false,
    )
}
