//! The five tray glyphs, on disk where a desktop can load them (S8).
//!
//! `IconName` + `IconThemePath` is what lets the DESKTOP draw the icon — at its own panel size, and
//! in its own colour. That is the property `10-tray.md` asks for ("Ship symbolic SVGs; the theme
//! recolours them") and the one `tray-icon` cannot offer at all: its Linux backend rasterises an
//! RGBA buffer to a temp PNG, and a PNG has already chosen its colours.
//!
//! Verified on a live Plasma session rather than assumed. An item pointed at an SVG whose
//! `#current-color-scheme` block declared `#ff00ff` rendered **white** in the panel, at the same
//! pixels where a plain magenta SVG had rendered magenta a minute earlier. The Breeze convention
//! works, so the mono forms are what ship.
//!
//! # Why they are written out rather than referenced in place
//!
//! `IconThemePath` is read by another process. Pointing it at wherever this binary happens to live
//! ties the tray to the install layout — it would work from a cargo target directory, from a
//! `.deb`, and not from a Flatpak or an AppImage, each failing as a blank tray icon with no error.
//! Embedding the five files and writing them to the runtime directory costs about 3 KB and works
//! the same everywhere. `tray-icon` writes to `$XDG_RUNTIME_DIR/tray-icon` for the same reason.

#![cfg(target_os = "linux")]

use std::io;
use std::path::{Path, PathBuf};

/// The five files, embedded. GENERATED — `gui/tools/render-tray-glyphs.mjs` produces them from the
/// marks on `10a Glyph states`, the same nodes the fidelity gate compares, and `npm run
/// glyphs:check` fails the build when they fall behind `ui/hexagon.js`.
const GLYPHS: &[(&str, &str)] = &[
    (
        "proton-sync-uptodate-symbolic",
        include_str!("../icons/tray/proton-sync-uptodate-symbolic.svg"),
    ),
    (
        "proton-sync-syncing-symbolic",
        include_str!("../icons/tray/proton-sync-syncing-symbolic.svg"),
    ),
    (
        "proton-sync-attention-symbolic",
        include_str!("../icons/tray/proton-sync-attention-symbolic.svg"),
    ),
    (
        "proton-sync-paused-symbolic",
        include_str!("../icons/tray/proton-sync-paused-symbolic.svg"),
    ),
    (
        "proton-sync-offline-symbolic",
        include_str!("../icons/tray/proton-sync-offline-symbolic.svg"),
    ),
];

/// Which glyph a daemon state wears.
///
/// **The same five and no more.** `10-tray.md`: "Only five forms exist. A solid filled hexagon is
/// not a state — it was drawn that way by mistake during design and corrected." The frontend's
/// `TRAY_GLYPH_STATES` names the same five, and `derive_state`'s six variants collapse onto them
/// here exactly as `screens/tray.js` collapses them for the panel: an expired session shares the
/// struck mark with an unreachable daemon (11-notifications.md puts an outage and an expired session
/// behind one icon), and a daemon that has never synced wears the needs-you form rather than the
/// settled one, because it has not synced anything.
pub fn glyph_for(state: gui_core::state::DaemonState) -> &'static str {
    use gui_core::state::DaemonState::*;
    match state {
        Running => "proton-sync-syncing-symbolic",
        Idle => "proton-sync-uptodate-symbolic",
        Paused => "proton-sync-paused-symbolic",
        AuthExpired | Unreachable => "proton-sync-offline-symbolic",
        FirstRun => "proton-sync-attention-symbolic",
    }
}

/// Where the SVGs are written, and what `IconThemePath` points at.
///
/// A flat directory of `<name>.svg`, which is what the spike proved a host resolves — not a
/// `hicolor/symbolic/apps` tree. Under `$XDG_RUNTIME_DIR` so it is cleaned up with the session and
/// is not world-readable.
pub fn theme_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("proton-sync-tray")
}

/// Write the five out. Called once at startup, before the item is published — a host reads
/// `IconName` as soon as it is registered, and a name that does not resolve yet is a blank icon that
/// only a second state change would repair.
pub fn install() -> io::Result<PathBuf> {
    let dir = theme_dir();
    std::fs::create_dir_all(&dir)?;
    for (name, body) in GLYPHS {
        write_if_changed(&dir.join(format!("{name}.svg")), body)?;
    }
    Ok(dir)
}

/// Rewriting an identical file would be harmless for us and not for the host: some watch the theme
/// directory and reload on any change, so an unconditional write at every startup is a flicker.
fn write_if_changed(path: &Path, body: &str) -> io::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|current| current == body) {
        return Ok(());
    }
    std::fs::write(path, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui_core::state::DaemonState;

    #[test]
    fn every_daemon_state_has_a_glyph_that_ships() {
        // A name with no file behind it is a blank tray icon — the failure mode with no error
        // message anywhere, in the one surface of the app nobody inspects.
        let shipped: Vec<&str> = GLYPHS.iter().map(|(name, _)| *name).collect();
        for state in [
            DaemonState::Running,
            DaemonState::Idle,
            DaemonState::Paused,
            DaemonState::AuthExpired,
            DaemonState::Unreachable,
            DaemonState::FirstRun,
        ] {
            assert!(
                shipped.contains(&glyph_for(state)),
                "{state:?} maps to {}, which is not one of the five files",
                glyph_for(state)
            );
        }
    }

    #[test]
    fn a_daemon_that_has_never_synced_does_not_wear_the_settled_glyph() {
        // The same false all-clear `screens/tray.js` refuses in the panel: a hollow hexagon is the
        // resting shape, and a daemon that has never copied a file is not resting.
        assert_ne!(
            glyph_for(DaemonState::FirstRun),
            glyph_for(DaemonState::Idle)
        );
    }

    #[test]
    fn the_five_files_are_symbolic_and_recolourable() {
        // What makes the desktop able to recolour them, and the reason the mono forms are what
        // ship. A file that lost `currentColor` would render in the design's own foreground on
        // every panel — including a light one, where it would be invisible.
        for (name, body) in GLYPHS {
            assert!(
                body.contains("currentColor"),
                "{name} has no currentColor — the desktop cannot recolour it"
            );
            assert!(
                body.contains("current-color-scheme"),
                "{name} is missing the Breeze stylesheet block"
            );
        }
    }

    #[test]
    fn no_glyph_animates() {
        // `renderHexagon` puts `animation:hexup 2.4s linear infinite` in a style attribute for the
        // syncing mark, and the generator strips it: nothing animates a tray icon and the SNI
        // protocol has no notion of one. A file that kept it would not move — it would just carry a
        // declaration no icon loader reads. DEVIATIONS §82e.
        for (name, body) in GLYPHS {
            assert!(!body.contains("animation"), "{name} still carries an animation");
        }
    }
}
