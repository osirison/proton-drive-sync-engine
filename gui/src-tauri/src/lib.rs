//! Tauri shell entrypoint. The shell owns no sync logic; it wires the fixed command surface
//! (thin wrappers over `gui-core`) to the webview and manages the resolved runtime paths.

mod commands;
mod config_path;
// Not Linux-gated even though its whole body is: every caller wants "raise this window" and none of
// them wants to know which display server is under it.
mod focus;
mod notify;
mod panel;
mod tray;
// The rows every tray menu draws, in one table. Not Linux-gated: the fallback tray is built on every
// platform, and the table is what stops the three surfaces drawing three menus.
mod tray_menu;

// The tray item, its menu and its glyph theme are the D-Bus half of S8 and exist on Linux alone —
// the only platform this app targets today (the engine's IPC is Unix-socket only), but the split
// keeps the `cfg` at the module boundary instead of scattered through `tray.rs`.
#[cfg(target_os = "linux")]
mod dbusmenu;
#[cfg(target_os = "linux")]
mod icons;
#[cfg(target_os = "linux")]
mod sni;

use std::sync::Mutex;
use tauri::Manager;

/// Make the window's identity the launcher's, so the desktop can find its icon.
///
/// GTK3 takes the Wayland `app_id` from `g_get_prgname()` (argv[0] — `proton-sync-gui`) and the X11
/// `WM_CLASS` from `(prgname, program class)`. Neither matched
/// `app.protondrivesync.engine.desktop`, so KWin drew `QIcon::fromTheme("wayland")` instead.
///
/// Must precede `Builder::run`: `gtk_init` fills an unset prgname from argv[0], the app_id is
/// stamped at toplevel creation, and the config's windows are built before the `setup` hook.
///
/// `gtk::init` only satisfies gdk-rs's initialized-GDK assert (the C call needs none). It is tao's
/// own first act and idempotent; no display leaves the X11 half unset and tao reports as before.
#[cfg(target_os = "linux")]
fn adopt_launcher_identity(identifier: &str) {
    gtk::glib::set_prgname(Some(identifier));
    if gtk::init().is_ok() {
        gtk::gdk::set_program_class(identifier);
    }
}

/// Take tao's titlebar off a window that wants the compositor's own.
///
/// tao 0.35 hangs a `GtkHeaderBar` inside a `GtkEventBox` on **every** Wayland window
/// (`platform_impl/linux/wayland/header.rs`), which forces GTK client-side decorations. KWin then
/// draws nothing, and that header's buttons are inert: the event box is `above_child` with no
/// handler, so per `GtkEventBox`'s contract every click inside it goes to the box and dies there
/// instead of reaching minimize or close. tao 0.37 deleted the module and keeps an empty titlebar
/// for *undecorated* windows only; this is that change applied from outside, and it self-disables
/// on the upgrade, when `titlebar()` is already `None`.
///
/// Only for a window that wants decorations. The tray panel is undecorated and must keep its
/// titlebar, or the compositor decorates the popover.
///
/// Must precede realize: measured, a 500x300 window done afterwards warns and comes out 590x466.
#[cfg(target_os = "linux")]
fn drop_toolkit_titlebar(window: &tauri::WebviewWindow) {
    use gtk::prelude::GtkWindowExt;

    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };
    if gtk_window.titlebar().is_some() {
        gtk_window.set_titlebar(None::<&gtk::Widget>);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Not a literal: this is `tauri.conf.json`'s `identifier`, and a second spelling here is a
    // second thing to keep in step with the launcher.
    let context = tauri::generate_context!();
    #[cfg(target_os = "linux")]
    adopt_launcher_identity(&context.config().identifier);

    tauri::Builder::default()
        // Settings › Folders' `Choose…`. Registered for the Rust side only: `commands::choose_folder`
        // calls it, and an app-defined command needs no capability grant — so the webview never gets
        // a file dialog it could open on its own.
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(config_path::RuntimePaths::resolve()))
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::pause,
            commands::resume,
            commands::sync_now,
            commands::resync,
            commands::approve,
            commands::deny,
            commands::keep,
            commands::list_pending_deletions,
            commands::read_config,
            commands::write_config,
            commands::choose_folder,
            commands::run_dry_run,
            commands::apply_plan,
            commands::scan_conflicts,
            commands::resolve_conflict,
            commands::read_conflict_pair,
            commands::path_sync_status,
            commands::search_files,
            commands::start_service,
            commands::restart_service,
            commands::send_notification,
            commands::close_notification,
            commands::read_notify_policy,
            commands::write_notify_policy,
            // The Phase-1 capability commands (C2/C4/C5) — data the design assumes and the daemon
            // does not expose. Added by the C-tasks, not by the screens that consume them.
            commands::free_space,
            commands::check_cli,
            commands::skip_rule_usage,
            commands::probe_folder,
            // The openers (#220/#231): the one capability behind `Open both in an editor`,
            // `Open folder`, `Open on Proton Drive` and `Open the system log`. No plugin and no
            // capability grant — these are app-defined commands shelling `xdg-open`, so the webview
            // gets exactly these four doors out and no general "open anything" permission.
            commands::open_paths,
            commands::open_folder,
            commands::open_remote,
            commands::open_system_log,
            // F4's keyboard map: Ctrl W and Ctrl Q. Same two paths the tray menu already offers.
            commands::close_window,
            commands::quit_app,
            // The tray panel (S8): its rows, and the two things only the webview knows — how tall it
            // came out, and when Esc was pressed.
            commands::tray_action,
            commands::resize_tray_panel,
            commands::hide_tray_panel,
        ])
        .setup(|app| {
            // Both are Linux-only types, and `#[cfg]` binds to ONE statement — the notifier's line
            // was outside the attribute above it and would have failed to compile off Linux.
            #[cfg(target_os = "linux")]
            {
                app.manage::<sni::SniState>(std::sync::Arc::new(tokio::sync::Mutex::new(None)));
                // The notification connection is made on first send (S9), so this starts empty.
                app.manage::<notify::NotifierState>(std::sync::Arc::new(tokio::sync::Mutex::new(
                    None,
                )));
            }
            // `main` is configured `"visible": false` so this lands before the window is realized,
            // and is shown here. Nothing else reads its start-up visibility — `tray::show_window`
            // already calls `show`/`unminimize` on every path that opens it.
            //
            // That flag carries a second effect, measured by replaying tao's construction order:
            // created visible, its `resize(1040, 764)` is the CSD-inclusive size, so the header came
            // out of the middle and the webview got 1040x716 — against frames drawn at 764 and a
            // gate that renders at 764 (`gui/tools/fidelity/assert.mjs`). Created hidden it is
            // 1040x764, with or without the header. The tao 0.37 upgrade ends the coupling: no
            // header is installed, so a window built visible measures 764 too.
            if let Some(main) = app.get_webview_window("main") {
                #[cfg(target_os = "linux")]
                drop_toolkit_titlebar(&main);
                let _ = main.show();
            }
            tray::setup(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                // Closing the MAIN window hides it to the tray rather than exiting, so the indicator
                // survives while syncing continues. A real exit is the tray's `Quit`, which since S8
                // also stops the daemon — that is what its `stops syncing` sub-label promises, and
                // `Close window · keeps syncing` is this path.
                tauri::WindowEvent::CloseRequested { api, .. }
                    if window.label() != panel::LABEL =>
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // THE PANEL GOES AWAY WHEN YOU LOOK AWAY, and this is the half of that which cannot
                // live in the webview: a click on another window never reaches our DOM.
                //
                // It is also why the panel TAKES focus when the indicator is clicked. The plan's two
                // sub-risks — "must not steal focus" and "must not linger after blur" — are the same
                // sentence twice if read literally: a window that never focuses never blurs, so it
                // could only be dismissed by clicking it, which is the lingering the other half
                // forbids. Taking focus from an explicit click is not stealing it; the thing being
                // ruled out is a panel that raises itself over someone's work unbidden, and nothing
                // here opens except on `Activate`.
                //
                // A BLUR ONLY MEANS SOMETHING IF THE WINDOW EVER HAD FOCUS, and the first version
                // did not check: driving `Activate` over the bus showed the panel and hid it again
                // in the same breath, because the compositor's focus-stealing prevention handed
                // focus straight back to whatever had it and the resulting `Focused(false)` was
                // indistinguishable from the user clicking away. The panel existed, unmapped,
                // looking exactly like a panel that had never opened.
                //
                // So `hide` waits for a blur that follows a focus. When the compositor never grants
                // focus at all the panel stays up, which is the right way round to be wrong: Esc,
                // any menu row, and a second click on the indicator all still dismiss it.
                tauri::WindowEvent::Focused(focused) if window.label() == panel::LABEL => {
                    if *focused {
                        panel::mark_focused();
                    } else if panel::take_focused() {
                        let _ = window.hide();
                    }
                }
                _ => {}
            }
        })
        .run(context)
        .expect("error while running proton-sync-gui");
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn repo_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    fn tauri_config() -> serde_json::Value {
        let text = std::fs::read_to_string(repo_path("gui/src-tauri/tauri.conf.json"))
            .expect("tauri.conf.json");
        serde_json::from_str(&text).expect("valid JSON")
    }

    fn identifier() -> String {
        tauri_config()["identifier"]
            .as_str()
            .expect("identifier is a string")
            .to_owned()
    }

    /// `drop_toolkit_titlebar` has to land before the window is realized, and the only thing
    /// delaying realize is this flag — which JSON gives no room to explain, in a file no other gate
    /// reads. Measured by replaying tao 0.35's construction order: dropped after realize, the
    /// window comes out 90px over on each axis and GTK warns to the journal, not to a terminal. The
    /// flag also decides the webview's height (1040x716 visible, 1040x764 hidden). The label is the
    /// other half — `lib.rs`, `tray.rs` and `commands.rs` all reach this window by that literal, and
    /// nothing else shows it, so a relabel now costs the window rather than a tray row.
    #[test]
    fn the_main_window_starts_hidden_so_its_titlebar_is_dropped_before_realize() {
        let config = tauri_config();
        let main = config["app"]["windows"]
            .as_array()
            .expect("app.windows")
            .iter()
            .find(|w| w["label"] == "main")
            .expect("a window labelled `main`");

        assert_eq!(
            main["visible"],
            serde_json::json!(false),
            "`main` must start hidden: `drop_toolkit_titlebar` runs in `setup`, and Tauri realizes \
             every window configured visible before that hook"
        );
    }

    /// `adopt_launcher_identity` makes the runtime app_id the config's `identifier`, which only
    /// resolves to an icon while a launcher of that exact name declares it. Nothing at build time
    /// links the two, so the drift is silent: the window falls back to breeze's `wayland` glyph and
    /// every gate stays green.
    #[test]
    fn the_launcher_is_named_for_the_identifier_the_window_announces() {
        let identifier = identifier();

        for launcher in [
            repo_path(&format!("packaging/freedesktop/{identifier}.desktop")),
            // setup.sh writes its own copy, with an absolute `Exec`; the identity lines must match.
            repo_path("setup.sh"),
        ] {
            let text = std::fs::read_to_string(&launcher)
                .unwrap_or_else(|e| panic!("{}: {e}", launcher.display()));
            for line in [
                format!("Icon={identifier}"),
                format!("StartupWMClass={identifier}"),
            ] {
                // Whole line, not a substring: `#Icon=…` is a comment and `Icon=…-v2` is another
                // icon, and both contain the needle. `trim` because a heredoc may be indented.
                assert!(
                    text.lines().any(|l| l.trim() == line),
                    "{} does not carry `{line}`",
                    launcher.display()
                );
            }
        }

        // setup.sh's install path, and uninstall.sh's removal path, name the file itself.
        for script in ["setup.sh", "upgrade.sh", "uninstall.sh"] {
            let text = std::fs::read_to_string(repo_path(script)).expect(script);
            assert!(
                text.contains(&format!("{identifier}.desktop")),
                "{script} does not name `{identifier}.desktop`"
            );
        }
    }

    /// The `desktop-entry` hint is the same fact as the app_id: it tells the notification server
    /// which launcher the banner belongs to.
    #[test]
    fn the_notification_desktop_entry_hint_is_that_same_identifier() {
        let source =
            std::fs::read_to_string(repo_path("gui/src-tauri/src/notify.rs")).expect("notify.rs");
        assert!(
            source.contains(&format!("\"{}\"", identifier())),
            "notify.rs's `desktop-entry` hint has drifted from tauri.conf.json's identifier"
        );
    }

    /// DEVIATIONS §103 (#221): a decision to shrink the main window for `3a Conflicts cleared`
    /// (and grow it back) was taken, then reversed 3m10s later (00:19:57Z → 00:23:07Z) in the same issue thread —
    /// the shell never resizes itself, for that state or the two others that ask the same question
    /// (`4a Empty`, `5a Checking`). Pinned against a regression the way `known-deviations.mjs`
    /// alone cannot: that file records what the fidelity gate compares, which is a headless page at
    /// a fixed 1040×764 viewport regardless of what `tauri.conf.json` says, so a real resize there
    /// would be invisible to it. This reads the config the running app actually uses.
    #[test]
    fn the_main_window_stays_a_fixed_1040x764_and_is_not_user_resizable() {
        let config = tauri_config();
        let main = config["app"]["windows"]
            .as_array()
            .expect("app.windows")
            .iter()
            .find(|w| w["label"] == "main")
            .expect("a window labelled `main`");

        assert_eq!(
            main["width"],
            serde_json::json!(1040),
            "§103: nothing app-driven may shrink the main window's declared width for any screen state"
        );
        assert_eq!(
            main["height"],
            serde_json::json!(764),
            "§103: nothing app-driven may shrink the main window's declared height for any screen state"
        );
        assert_eq!(
            main["resizable"],
            serde_json::json!(false),
            "§103 settles that the shell does not resize itself — it does not also make the window \
             user-resizable, which stays a separate, still-open question (#273)"
        );
    }

    /// The operative sentence of §103's corrected decision: "`tauri::Window::set_size` is not
    /// called on [the main window] for this state." Adversarial review found two escapes in an
    /// earlier version of this guard that only checked "which file calls `set_size`": (a) a
    /// non-recursive `read_dir` never visited a new subdirectory, and (b) the file-only check
    /// could not tell `panel::resize`'s existing, legitimate call (the tray panel, which asks to be
    /// always-on-top and has no fixed size of its own) apart from a NEW call added inside that same
    /// file but
    /// targeting the main window instead — the exact shape the reversed first decision proposed,
    /// and the file someone reaching for "add a resize" would most plausibly reach for, since it
    /// already imports the right types.
    ///
    /// This closes (a) by walking `src/` recursively. It closes (b) by proxy rather than real
    /// data-flow analysis, because that is what source text can support: every existing call site
    /// that resizes or otherwise touches the main window names it by its literal quoted label,
    /// `"main"` (`lib.rs`, `tray.rs`, `commands.rs`) — `panel.rs` contains that literal NOWHERE
    /// today, its window coming only from `panel::LABEL`. So a file gaining BOTH a `set_size` call
    /// AND the `"main"` literal is what "resize main from wherever" looks like in source, and is
    /// flagged independently of whether that file was already an allowed caller.
    ///
    /// WHAT THIS DOES NOT CATCH — a guard whose comment overstates it is worse than a weak guard
    /// that says so: renaming the label so `"main"` is never spelled at the call site; reaching
    /// main through an alias, a struct field, or a closure capture rather than a fresh
    /// `get_webview_window("main")` in the same file; a `WebviewWindowBuilder::inner_size` override
    /// that never calls `set_size` at all (the `tauri.conf.json` test above only pins the
    /// *configured* size, not a builder call that ignores it); and a webview-driven resize through
    /// Tauri's JS `core:window` capability — closed today (`capabilities/default.json` grants only
    /// `core:default`/`core:event:default`, no `allow-set-size`), but nothing here or in the JS
    /// tests asserts that it stays closed.
    #[test]
    fn set_size_never_reaches_the_main_window() {
        fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("readable src dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    rust_files(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    out.push(path);
                }
            }
        }

        // Split so this test's own source contains neither needle contiguously — a plain literal
        // would match `lib.rs` itself, both as a false "caller" and a false "names main".
        let set_size_needle = ["set_", "size("].concat();
        let main_needle = ["\"", "main", "\""].concat();

        let src_dir = repo_path("gui/src-tauri/src");
        let mut files = Vec::new();
        rust_files(&src_dir, &mut files);

        let mut callers: Vec<String> = Vec::new();
        let mut names_main_too: Vec<String> = Vec::new();
        for path in &files {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            let relative = path
                .strip_prefix(&src_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned();
            if text.contains(&set_size_needle) {
                callers.push(relative.clone());
                if text.contains(&main_needle) {
                    names_main_too.push(relative);
                }
            }
        }
        callers.sort();

        assert_eq!(
            callers,
            vec!["panel.rs".to_string()],
            "`set_size` must stay confined to the tray panel, wherever in the tree it is called \
             from — §103 pins the main window as never resized by the app"
        );
        assert!(
            names_main_too.is_empty(),
            "{names_main_too:?} calls `set_size` in a file that also names the main window by its \
             `\"main\"` label — §103 forbids resizing main, and this is what that looks like in \
             source text even inside a file already licensed to resize the tray panel"
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod identity_tests {
    /// Gutting `adopt_launcher_identity` left the whole suite green: the guards above check what the
    /// launcher declares, and nothing checked that the process ever announces it. prgname IS the
    /// Wayland app_id, so this is that half. The X11 half needs a display and is skipped headlessly.
    #[test]
    fn adopting_the_identity_sets_the_name_the_compositor_reads() {
        let restore = gtk::glib::prgname();
        super::adopt_launcher_identity("app.test.launcher-identity");
        let announced = gtk::glib::prgname();
        gtk::glib::set_prgname(restore.as_deref());
        assert_eq!(announced.as_deref(), Some("app.test.launcher-identity"));
    }
}
