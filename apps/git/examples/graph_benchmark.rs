//! Reproducible, local-only history/layout/cache benchmark. Never touches user repos.
use nixe_git::{
    core::history::{Cursor, Scope},
    graph::virtualization::{MAX_CACHE, visible_range},
};
use std::{
    io::Write,
    process::{Command, Stdio},
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
fn main() -> anyhow::Result<()> {
    let count = std::env::args().nth(1).and_then(|s| s.parse::<usize>().ok()).unwrap_or(100000);
    let dir = tempfile::tempdir()?;
    gix::init(dir.path())?;
    let mut input = String::with_capacity(count * 150);
    for i in 1..=count {
        let message = format!("Commit {i}\n");
        input.push_str(&format!(
            "commit refs/heads/main\nmark :{i}\ncommitter Nixe Benchmark <bench@example.invalid> {} +0000\ndata {}\n{}",
            1600000000 + i,
            message.len(),
            message
        ));
        if i > 1 {
            input.push_str(&format!("from :{}\n", i - 1));
        }
        input.push('\n');
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(input.as_bytes())?;
    drop(input);
    let output = child.wait_with_output()?;
    anyhow::ensure!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut repo = gix::discover(dir.path())?;
    repo.object_cache_size_if_unset(8 * 1024 * 1024);
    let started = Instant::now();
    let mut cursor = Cursor::new(&repo, Scope::All, Arc::new(AtomicBool::new(false)))?;
    cursor.fill(128)?;
    let first = started.elapsed();
    let rows = cursor.store.page(0, MAX_CACHE)?;
    drop(rows);
    cursor.fill(count + 1)?;
    let full = started.elapsed();
    anyhow::ensure!(cursor.store.len() == count && cursor.done, "History count mismatch");
    let started = Instant::now();
    let mut maximum = 0;
    for page in 0..100 {
        let offset = (page * count / 100) as f64 * 36.0;
        let range = visible_range(offset, 800.0, 36.0, count);
        let rows = cursor.store.page(range.start, range.len())?;
        maximum = maximum.max(rows.len());
    }
    let pages = started.elapsed();
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).map(String::from))
        .unwrap_or_default();
    println!(
        "commits={count} first_128_ms={:.1} all_layout_ms={:.1} cached_page_avg_ms={:.3} max_viewport_rows={maximum} ui_cache_limit={MAX_CACHE} worker_{rss}",
        first.as_secs_f64() * 1000.0,
        full.as_secs_f64() * 1000.0,
        pages.as_secs_f64() * 10.0
    );
    Ok(())
}
