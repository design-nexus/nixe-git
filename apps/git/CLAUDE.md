# Git — notes for working on this app

- GTK4 + GtkSourceView 5, no libadwaita, built to `STYLE.md`. App id
  `io.github.design_nexus.nixe.Git`; settings in `~/.config/nixe-git/settings.toml`.
- `src/core` (the `nixe_git` library) has no GTK. System Git is the only writer:
  every mutation is an `operations::Operation` run through `operations::run`/`invoke`
  (structured arguments, literal pathspecs, common-directory locks in `jobs`).
  gitoxide reads HEAD, refs and history. `history::Session` is the graph's worker.
- `src/app.rs`: `Ui::open` (user: disables actions while reading), `Ui::reload` (file
  watcher: quiet, may reuse the watch plan) and `execute_at_then` for operations.
  `loads` drops a status read that an operation overtook.
- `src/ui/window.rs` builds the window and owns `apply` (in-place file list), the toolbar
  (Fetch/Pull/Push, repository menu) and notices (`toast` fades; `error` stays).
  `ui::menu`/`popup_at`/`menu_button` are the app's menus; `ui::dev` holds the
  `NIXE_GIT_SNAPSHOT`/`NIXE_GIT_PREVIEW_*` developer previews.
- **Control** (`src/control.rs`): `repo.open`, `repo.status`, `repo.log`, `repo.fetch`
  (`repo.`, as the Assistant's own `git.status` already exists). Nothing commits,
  pushes, discards or rewrites history.
- Checks: `cargo test -p nixe-git` (temporary repositories only; tests mustn't depend on
  the user's Git config), `cargo clippy -p nixe-git --all-targets` is clean.
- Test window with scratch data: `XDG_CONFIG_HOME=… XDG_DATA_HOME=… XDG_CACHE_HOME=…
  XDG_STATE_HOME=… ./target/debug/nixe-git REPO`; `NIXE_GIT_SNAPSHOT=out.png` captures
  the frame and quits (wrap in `dbus-run-session --` to keep it off the real bus name).
