//! Worker-local operation control, bounded progress and common-directory locking.
use anyhow::{Context, Result, bail, ensure};
use std::{
    cell::RefCell,
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Debug)]
pub struct Control {
    allowed: bool,
    cancel: AtomicBool,
    progress: Mutex<String>,
}
impl Control {
    pub fn new(cancellable: bool) -> Arc<Self> {
        Arc::new(Self { allowed: cancellable, cancel: AtomicBool::new(false), progress: Mutex::new(String::new()) })
    }
    pub fn cancel(&self) -> bool {
        if !self.allowed {
            return false;
        }
        self.cancel.store(true, Ordering::Release);
        true
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
    pub fn cancellable(&self) -> bool {
        self.allowed
    }
    pub fn progress(&self) -> String {
        self.progress.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn update(&self, value: &str) {
        *self.progress.lock().unwrap_or_else(|e| e.into_inner()) =
            value.chars().filter(|c| !c.is_control() || *c == '\t').take(240).collect();
    }
    fn check(&self) -> Result<()> {
        ensure!(
            !self.cancelled(),
            "Stopped on request. Review repository state and any partial clone directory before retrying."
        );
        Ok(())
    }
}
thread_local! {static CURRENT:RefCell<Option<Arc<Control>>>=const{RefCell::new(None)};static HELD:RefCell<HashSet<PathBuf>>=RefCell::new(HashSet::new());}
struct Scope(Option<Arc<Control>>);
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|c| *c.borrow_mut() = self.0.take());
    }
}
pub fn controlled<T>(control: Arc<Control>, work: impl FnOnce() -> Result<T>) -> Result<T> {
    control.check()?;
    let previous = CURRENT.with(|c| c.borrow_mut().replace(control));
    let _scope = Scope(previous);
    work()
}
pub fn context() -> Option<Arc<Control>> {
    CURRENT.with(|c| c.borrow().clone())
}
struct Lock {
    key: PathBuf,
    _file: File,
}
impl Drop for Lock {
    fn drop(&mut self) {
        HELD.with(|h| {
            h.borrow_mut().remove(&self.key);
        });
    }
}
pub fn locked<T>(root: &Path, work: impl FnOnce() -> Result<T>) -> Result<T> {
    let key = gix::discover(root)?.common_dir().canonicalize()?;
    if HELD.with(|h| h.borrow().contains(&key)) {
        return work();
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(key.join("nixe-operation.lock"))
        .context("Could not open the Nixe operation lock")?;
    ensure!(file.metadata()?.is_file(), "The Nixe operation lock is not a regular file");
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {
                if let Some(control) = context().filter(|c| c.cancellable()) {
                    control.check()?;
                    control.update("Waiting for another Nixe Git operation…");
                    thread::sleep(Duration::from_millis(50));
                } else {
                    bail!(
                        "Another Nixe Git process is changing this repository or one of its worktrees. Wait for it to finish, then retry."
                    );
                }
            }
            Err(error) => bail!("Could not lock this repository: {error}"),
        }
    }
    HELD.with(|h| {
        h.borrow_mut().insert(key.clone());
    });
    let _lock = Lock { key, _file: file };
    work()
}
struct Captured {
    bytes: Vec<u8>,
    overflow: bool,
}
fn drain(mut pipe: impl Read, limit: usize, control: Option<Arc<Control>>, tail: bool) -> std::io::Result<Captured> {
    let mut bytes = Vec::new();
    let mut pending = Vec::new();
    let mut overflow = false;
    let mut chunk = [0; 8192];
    loop {
        let size = pipe.read(&mut chunk)?;
        if size == 0 {
            break;
        }
        let data = &chunk[..size];
        if tail {
            if bytes.len() + size > limit {
                let discard = (bytes.len() + size - limit).min(bytes.len());
                bytes.drain(..discard);
                overflow = true;
            }
            bytes.extend_from_slice(&data[data.len().saturating_sub(limit)..]);
        } else {
            let keep = (limit - bytes.len()).min(size);
            bytes.extend_from_slice(&data[..keep]);
            overflow |= keep < size;
        }
        if let Some(control) = &control {
            for b in data {
                if matches!(*b, b'\r' | b'\n') {
                    if !pending.is_empty() {
                        let line = String::from_utf8_lossy(&pending);
                        if [
                            "remote:",
                            "Receiving objects:",
                            "Resolving deltas:",
                            "Counting objects:",
                            "Compressing objects:",
                            "Writing objects:",
                            "Enumerating objects:",
                            "Updating files:",
                            "Checking out files:",
                        ]
                        .iter()
                        .any(|p| line.starts_with(p))
                        {
                            control.update(&line);
                        }
                        pending.clear();
                    }
                } else if pending.len() < 1024 {
                    pending.push(*b);
                }
            }
        }
    }
    Ok(Captured { bytes, overflow })
}
/// Child groups isolate cancellation from the application and unrelated processes.
pub(crate) fn output(mut command: Command, label: &str, input: Option<Vec<u8>>) -> Result<Output> {
    let control = context();
    if let Some(c) = &control {
        c.check()?;
        c.update(label);
    }
    command
        .process_group(0)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("Could not start system Git; install the git package")?;
    let pid = child.id() as i32;
    let writer = input.map(|bytes| {
        let mut stdin = child.stdin.take().unwrap();
        thread::spawn(move || stdin.write_all(&bytes))
    });
    let stdout = child.stdout.take().unwrap();
    let out = thread::spawn(move || drain(stdout, 128 * 1024 * 1024, None, false));
    let stderr = child.stderr.take().unwrap();
    let progress = control.clone();
    let err = thread::spawn(move || drain(stderr, 256 * 1024, progress, true));
    let mut status = None;
    let mut interrupted: Option<Instant> = None;
    let mut signal = 0;
    loop {
        if control.as_ref().is_some_and(|c| c.cancelled()) {
            let start = *interrupted.get_or_insert_with(Instant::now);
            let next = if start.elapsed() >= Duration::from_secs(4) {
                libc::SIGKILL
            } else if start.elapsed() >= Duration::from_secs(2) {
                libc::SIGTERM
            } else {
                libc::SIGINT
            };
            if next != signal {
                signal = next;
                unsafe {
                    libc::kill(-pid, signal);
                }
            }
        }
        if status.is_none() {
            status = child.try_wait()?;
        }
        if status.is_some() && out.is_finished() && err.is_finished() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let stdout = out.join().map_err(|_| anyhow::anyhow!("Git output reader failed"))??;
    let stderr = err.join().map_err(|_| anyhow::anyhow!("Git progress reader failed"))??;
    let written = writer.map(|w| w.join());
    if interrupted.is_some() {
        bail!(
            "Stopped on request. Git may have completed part of this operation. Review refreshed repository state before retrying; partial clone directories are retained."
        );
    }
    let status = status.unwrap();
    if status.success() {
        if let Some(written) = written {
            written.map_err(|_| anyhow::anyhow!("Git input worker failed"))??;
        }
        ensure!(!stdout.overflow, "Git output exceeded 128 MiB. Inspect this result in an external tool.");
    }
    Ok(Output { status, stdout: stdout.bytes, stderr: stderr.bytes })
}
