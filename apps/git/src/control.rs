//! Git's tools on the control interface (`nixe_core::control`), for Nixe Assistant: open a
//! repository, read the open one's status and recent commits, and fetch. Nothing commits,
//! pushes, discards or rewrites history. (`repo.`, as the Assistant's own `git.status`
//! already reads repositories without the app.)

use crate::ui::window;
use gtk::prelude::*;
use nixe_core::control::{Reply, home_path, text};
use serde_json::{Value, json};

pub fn tools() -> Value {
    json!([
        {"name": "repo.open", "description": "Open a Git repository (a folder in the home folder) in Git.", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}},
        {"name": "repo.status", "description": "The repository open in Git: its branch, upstream, commits ahead and behind, and changed files.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "repo.log", "description": "Recent commits on the open repository's current branch: hash, author, date, subject.", "inputSchema": {"type": "object", "properties": {"count": {"type": "integer", "minimum": 1, "maximum": 50}}}},
        {"name": "repo.fetch", "description": "Fetch all remotes of the repository open in Git (downloads; changes no branch or file).", "inputSchema": {"type": "object", "properties": {}}},
    ])
}

fn status() -> Result<Value, String> {
    let ui = window::current().ok_or("Git isn't ready")?;
    let snapshot = ui.snapshot.borrow();
    let s = snapshot.as_ref().ok_or("no repository is open in Git")?;
    let files: Vec<Value> =
        s.status.files.iter().take(100).map(|f| json!({"path": f.path.to_string_lossy(), "state": f.kind()})).collect();
    Ok(json!({
        "repository": s.repository.root.to_string_lossy(),
        "branch": s.repository.branch,
        "upstream": s.status.upstream,
        "ahead_behind": s.status.divergence,
        "changed": s.status.files.len(),
        "files": files,
        "untrusted_content": true,
    }))
}

/// `git log` of the current branch, read off the main thread.
fn log(count: u64, reply: Reply) {
    let Some(root) = window::current().and_then(|ui| ui.root()) else {
        return reply(Err("no repository is open in Git".into()));
    };
    nixe_core::cmd::background(
        move || {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["--no-pager", "log", &format!("--max-count={count}"), "--format=%H%x00%an%x00%aI%x00%s%x00", "--"])
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
        },
        move |output| {
            let output = match output {
                Ok(o) if o.status.success() => o.stdout,
                Ok(o) => return reply(Err(String::from_utf8_lossy(&o.stderr).trim().to_string())),
                Err(e) => return reply(Err(e.to_string())),
            };
            let fields: Vec<String> = output.split(|b| *b == 0).map(|f| String::from_utf8_lossy(f).trim().to_string()).collect();
            let commits: Vec<Value> = fields
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| json!({"hash": c[0], "author": c[1], "date": c[2], "subject": c[3]}))
                .collect();
            reply(Ok(json!({"commits": commits, "untrusted_content": true})))
        },
    );
}

pub fn handle(name: &str, args: Value, reply: Reply) {
    let Some(ui) = window::current() else { return reply(Err("Git isn't ready".into())) };
    match name {
        "repo.open" => {
            let path = match text(&args, "path").and_then(|p| home_path(&p)) {
                Ok(p) if p.is_dir() => p,
                Ok(_) => return reply(Err("that folder doesn't exist".into())),
                Err(e) => return reply(Err(e)),
            };
            ui.window.present();
            ui.open(path.clone());
            reply(Ok(json!({"opening": path.to_string_lossy()})))
        }
        "repo.status" => reply(status()),
        "repo.log" => log(args["count"].as_u64().unwrap_or(10).clamp(1, 50), reply),
        "repo.fetch" => {
            let Some(root) = ui.root() else { return reply(Err("no repository is open in Git".into())) };
            ui.execute_at_then(root, nixe_git::core::operations::Operation::Fetch, move |result| {
                reply(result.map(|_| json!({"fetched": true})))
            });
        }
        _ => reply(Err(format!("unknown tool {name}"))),
    }
}
