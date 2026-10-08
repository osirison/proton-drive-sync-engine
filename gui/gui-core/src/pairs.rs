//! Which folder pair a request addresses, and which one the GUI has chosen (#102 phase 5a, ADR 0005
//! §4).
//!
//! Four pure pieces, none of which does I/O, so the decisions that can send a request to the wrong
//! folder are tested without a socket:
//!
//! - [`Target`] — the pair a control-socket request is addressed to, **as the wire spells it**.
//! - [`wire_selector`] — the one rule for turning a chosen pair into that spelling.
//! - [`resolve_selection`] — the one rule for validating a remembered choice against what exists.
//! - [`PairCapability`] — what the last reply says about whether the daemon understands a selector.
//!
//! **The default pair is addressed by omission.** A request that names no pair reaches the first
//! `[[pair]]` table (or the implicit pair called `default`), and that is what every client that
//! predates multi-pair sends for every verb. Keeping the default pair on that spelling is what makes
//! a one-pair setup send byte-identical requests to the ones it always sent, and what lets an older
//! daemon keep working: only a request for a *non-default* pair needs a daemon that understands the
//! field at all.

use crate::wire::{ControlRequest, ControlResponse};

/// The pair a control-socket request is addressed to, in the form it travels on the wire.
///
/// Not a pair *name*: [`Target::DEFAULT`] is an **absent** selector, and a [`Target::named`] pair is
/// a present one, so the type cannot be built from "the selected pair" without going through
/// [`wire_selector`] first — which is where the default pair becomes absent. A caller that put the
/// default pair's own name on the wire would send a request no client predating multi-pair could
/// make, and one a daemon predating the field would act on without ever having read the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Target<'a>(Option<&'a str>);

impl Target<'static> {
    /// No selector: the default pair. What every verb sent before multi-pair existed, and what
    /// every daemon-wide question (`shutdown`, "is anything there") sends.
    pub const DEFAULT: Self = Target(None);
}

impl<'a> Target<'a> {
    /// A pair addressed by name. Matched **byte-exactly** by the daemon (a request is never fuzzy
    /// about which folder it means), so the name must be one the daemon reported.
    pub fn named(name: &'a str) -> Self {
        Target(Some(name))
    }

    /// From an already-decided wire selector — the shape [`wire_selector`] returns.
    pub fn from_selector(selector: Option<&'a str>) -> Self {
        Target(selector)
    }

    /// The selector this puts on the wire: `None` for the default pair.
    pub fn selector(self) -> Option<&'a str> {
        self.0
    }

    /// `request`, addressed to this pair. The one place `ControlRequest::pair` is written by the
    /// GUI, so a request builder cannot forget it or write it twice.
    pub fn address(self, mut request: ControlRequest) -> ControlRequest {
        request.pair = self.0.map(str::to_owned);
        request
    }
}

/// The wire selector for the pair `selected`: absent when it is the default pair, its name
/// otherwise.
///
/// `default_name` is the name of the pair a request that names none reaches — the first pair the
/// daemon reported (or, before it has answered, the first one the config file declares). With one
/// pair, and with two where the first is chosen, the request is the one the app always sent.
pub fn wire_selector(selected: &str, default_name: &str) -> Option<String> {
    (selected != default_name).then(|| selected.to_owned())
}

/// The pair to show, given the one remembered and the ones that exist.
///
/// The remembered pair if it still exists, otherwise the first known pair (the default pair),
/// otherwise nothing. **Pure and re-run on every poll, and it never rewrites the remembered value**:
/// a daemon that is restarting, or running an older config, momentarily knows fewer pairs than the
/// user has, and erasing a good preference on that evidence would lose it for good. The answer is
/// borrowed from `known`, so what comes back is always a name the daemon actually reported.
pub fn resolve_selection<'a>(saved: Option<&str>, known: &[&'a str]) -> Option<&'a str> {
    saved
        .and_then(|saved| known.iter().copied().find(|name| *name == saved))
        .or_else(|| known.first().copied())
}

/// Whether the daemon behind the socket understands a pair selector, as the **last reply** showed.
///
/// A daemon that predates multi-pair silently ignores `ControlRequest::pair` and acts on its one
/// pair, which is the worst possible answer to a request aimed at another folder — so the capability
/// is read from what a reply *carries*, never assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PairCapability {
    /// No reply has been seen yet, or the last one proves neither way.
    #[default]
    Unknown,
    /// The reply carried neither `pair` nor `pairs`: a daemon older than the selector. Every request
    /// reaches its one pair whatever it says.
    Legacy,
    /// The reply listed the configured pairs. Never empty on a real reply, `N == 1` included, which
    /// is why a one-pair daemon that supports `--pair` is distinguishable from one that does not.
    MultiPair,
}

impl PairCapability {
    /// The capability a reply shows.
    ///
    /// - `pairs` non-empty → [`Self::MultiPair`]. This holds even when `pair` is `None`: that is a
    ///   daemon that understood the selector and found it names no pair, which is the strongest
    ///   evidence there is that it reads the field.
    /// - `pairs` empty and `pair` absent → [`Self::Legacy`].
    /// - `pairs` empty and `pair` present → [`Self::Unknown`]. No daemon sends that shape (a reply
    ///   that says which pair it describes also lists them), so it is evidence of nothing, and
    ///   guessing either way is how a selector reaches a daemon that ignores it.
    pub fn from_reply(response: &ControlResponse) -> Self {
        match (response.pairs.is_empty(), response.pair.is_some()) {
            (false, _) => Self::MultiPair,
            (true, false) => Self::Legacy,
            (true, true) => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{ControlCommand, PairSummary};
    use std::path::PathBuf;

    fn reply(pair: Option<&str>, pairs: &[&str]) -> ControlResponse {
        ControlResponse {
            status: "running".into(),
            paused: false,
            syncing: false,
            reconcile_seq: 0,
            pending_changes: 0,
            message: String::new(),
            last_sync_epoch_secs: Some(1),
            last_error: None,
            last_plan_summary: None,
            last_successful_sync_summary: None,
            status_history: vec![],
            pending_deletions: vec![],
            failed_items: vec![],
            failed_item_count: 0,
            config: None,
            activity: None,
            unsyncable: vec![],
            history: None,
            file_history: None,
            index_totals: None,
            listing: None,
            plan: None,
            apply: None,
            auth: Default::default(),
            pair: pair.map(str::to_owned),
            pairs: pairs
                .iter()
                .map(|name| PairSummary {
                    name: (*name).to_owned(),
                    local_root: PathBuf::from("/l"),
                    remote_root: PathBuf::from("/r"),
                    db_path: PathBuf::from("/d"),
                    paused: false,
                    syncing: false,
                    reconcile_seq: 0,
                    last_sync_epoch_secs: None,
                    last_error: None,
                    pending_changes: 0,
                    pending_deletions: 0,
                })
                .collect(),
        }
    }

    /// The default pair is addressed by OMITTING the selector (ADR 0005 §4). Everything that makes a
    /// one-pair setup send the requests it always sent rests on this one row.
    #[test]
    fn the_default_pair_is_addressed_by_omission() {
        assert_eq!(wire_selector("default", "default"), None);
        assert_eq!(wire_selector("photos", "photos"), None);
        assert_eq!(wire_selector("docs", "photos"), Some("docs".to_owned()));
        // Byte-exact, like the daemon: a different case is a different pair, not the default.
        assert_eq!(wire_selector("Photos", "photos"), Some("Photos".to_owned()));
    }

    #[test]
    fn a_target_writes_the_selector_into_the_request_and_nowhere_else() {
        let plain = ControlRequest::new(ControlCommand::Pause);
        assert_eq!(Target::DEFAULT.address(plain.clone()), plain);
        let addressed = Target::named("photos").address(plain.clone());
        assert_eq!(addressed.pair.as_deref(), Some("photos"));
        assert_eq!(
            ControlRequest {
                pair: None,
                ..addressed
            },
            plain,
            "only `pair` may differ"
        );
        // It overwrites: a request built with a stale selector is re-addressed, not merged.
        let stale = ControlRequest {
            pair: Some("old".to_owned()),
            ..ControlRequest::new(ControlCommand::Status)
        };
        assert_eq!(Target::DEFAULT.address(stale).pair, None);
        assert_eq!(Target::from_selector(Some("x")).selector(), Some("x"));
    }

    #[test]
    fn a_remembered_pair_that_still_exists_wins_and_otherwise_the_default_does() {
        let known = ["photos", "docs"];
        assert_eq!(resolve_selection(Some("docs"), &known), Some("docs"));
        // Gone, or never existed: the default pair, not an error and not a guess.
        assert_eq!(resolve_selection(Some("gone"), &known), Some("photos"));
        assert_eq!(resolve_selection(None, &known), Some("photos"));
        // Nothing known yet (a daemon that has not answered, no config): nothing to select.
        assert_eq!(resolve_selection(Some("docs"), &[]), None);
        // Byte-exact.
        assert_eq!(resolve_selection(Some("Docs"), &known), Some("photos"));
    }

    #[test]
    fn the_capability_is_read_from_what_a_reply_carries() {
        // A daemon that predates the selector: neither field.
        assert_eq!(
            PairCapability::from_reply(&reply(None, &[])),
            PairCapability::Legacy
        );
        // One pair is still multi-pair-capable: the list is never empty on a real reply.
        assert_eq!(
            PairCapability::from_reply(&reply(Some("default"), &["default"])),
            PairCapability::MultiPair
        );
        // An unresolved selector: the verb did nothing, but the daemon read the field.
        assert_eq!(
            PairCapability::from_reply(&reply(None, &["a", "b"])),
            PairCapability::MultiPair
        );
        // The shape no daemon sends says nothing either way.
        assert_eq!(
            PairCapability::from_reply(&reply(Some("default"), &[])),
            PairCapability::Unknown
        );
        assert_eq!(PairCapability::default(), PairCapability::Unknown);
    }
}
