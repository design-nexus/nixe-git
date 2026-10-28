# Git — notes for working on this app

- GTK4 + GtkSourceView 5, no libadwaita, built to `STYLE.md` on Nyx Design 2.0
  (`use_design_2`). App id `io.github.design_nexus.nyx.Git`; settings in
  `~/.config/nyx-git/settings.toml`.
- `src/core` (the `nyx_git` library) has no GTK. System Git is the only writer:
  every mutation is an `operations::Operation` run through `operations::run`/`invoke`
  (structured arguments, literal pathspecs, common-directory locks in `jobs`).
  gitoxide reads HEAD, refs and history. `history::Session` is the graph's worker.
- `src/app.rs`: `Ui::open` (user: disables actions while reading), `Ui::reload` (file
  watcher: quiet, may reuse the watch plan) and `execute_at_then` for operations.
  `loads` drops a status read that an operation overtook.
- `src/ui/window.rs` builds the window and owns `apply` (in-place file list), the header
  and notices (`toast` fades; `error` stays). The unified header: the app with the
  repository and branch as its subtitle and the sidebar toggle; History/Changes in the
  middle; what's changed (hidden under 980 px), Fetch/Pull/Push as icons with the
  upstream's counts (Push becomes Publish without an upstream), the repository menu,
  search, refresh and Settings. No status bar, no toolbar row; repository tabs sit
  under the header.
  `ui::menu`/`popup_at`/`menu_button` are the app's menus; `ui::dev` holds the
  `NYX_GIT_SNAPSHOT`/`NYX_GIT_PREVIEW_*` developer previews.
- **Control** (`src/control.rs`): `repo.open`, `repo.status`, `repo.log`, `repo.fetch`
  (`repo.`, as the Assistant's own `git.status` already exists). Nothing commits,
  pushes, discards or rewrites history.
- Checks: `cargo test -p nyx-git` (temporary repositories only; tests mustn't depend on
  the user's Git config), `cargo clippy -p nyx-git --all-targets` is clean.
- Test window with scratch data: `NYX_GIT_APP_ID=io.github.design_nexus.nyx.GitTest
  GIT_CONFIG_GLOBAL=/dev/null XDG_CONFIG_HOME=… XDG_DATA_HOME=… XDG_CACHE_HOME=…
  XDG_STATE_HOME=… ./target/debug/nyx-git REPO`, on a private headless output (never the
  desktop), captured with `grim -o`. Set `reduce_motion = true` in its settings: a
  headless output barely runs the frame clock, so a view's crossfade never finishes.
  The `NYX_GIT_PREVIEW_*` dialogs still need `NYX_GIT_SNAPSHOT`.
