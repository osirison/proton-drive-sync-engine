//! `proton-sync-gui-core` — the pure-Rust data layer for the Proton Drive Sync desktop GUI.
//!
//! The GUI is a **thin client**: it owns no sync logic and no index. This crate is the single
//! typed boundary between the UI and the daemon's real surface. It reads the control socket, the
//! JSON state sidecars, the config file, the conflict sidecars, and the dry-run plan — reusing
//! the engine's own wire types so the GUI can never drift from the daemon's on-disk / on-wire
//! JSON. It has **no GUI dependency** (no webkit/Tauri), so it builds and unit-tests standalone;
//! the Tauri command layer is a thin set of wrappers over these functions.
//!
//! ## Facade rule (keep this intact)
//! Everything downstream — the Tauri `#[command]`s and every screen — imports wire types from
//! [`wire`] in *this* crate, never from `proton_drive_sync_engine` directly. That preserves the
//! option to extract a standalone `proton-sync-types` crate later without touching a screen.
//!
//! ## Corrections this layer encodes (verified against the engine, not the design docs)
//! - Live state comes from the socket then the JSON sidecars (`*.metrics.json`, `*.status.json`),
//!   never live SQLite (the index runs without WAL → readers race the writer).
//! - `conflicts` / `destructive_actions` / `skipped_unsupported` live inside a **nullable**
//!   `PlanSummary`, not at the top level of the status reply.
//! - The typed-DELETE gate keys on [`wire::SyncAction::delete_direction`] — `remote_delete` /
//!   `local_delete` only. `purge` is *display*-destructive but is **never** gated (see [`plan`]).
//! - Conflict sidecars match **both** `*.proton-cloud.*` and the extensionless `*.proton-cloud`
//!   (see [`conflicts`]).
//! - The config writer edits in place (comments + daemon-only keys preserved) and refuses to
//!   write anything the daemon's own `deny_unknown_fields` parser would reject (see [`config_io`]).

pub mod config_io;
pub mod conflicts;
pub mod index_read;
pub mod ipc;
pub mod plan;
pub mod sidecars;
pub mod state;

/// The daemon's own wire/serialization types, re-exported so the GUI depends on this facade
/// rather than on `proton_drive_sync_engine` directly. See the crate-level "facade rule".
pub mod wire {
    pub use proton_drive_sync_engine::daemon::MetricsSnapshot;
    pub use proton_drive_sync_engine::index::{EntityKind, FileRecord};
    pub use proton_drive_sync_engine::ipc::{
        ControlCommand, ControlRequest, ControlResponse, PendingDeletion, StatusHistoryEntry,
    };
    pub use proton_drive_sync_engine::sync::{
        DeleteDirection, DryRunReport, PlanSummary, PlannedAction, SyncAction,
    };
}

pub use state::DaemonState;
