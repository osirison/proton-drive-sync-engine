//! Which folder pair runs next (#102 phase 4a, ADR 0005 §5).
//!
//! **Pure**: no tokio, no I/O, no clock reads. Every method that needs the time is handed `now`,
//! so the ordering is testable with synthetic instants and the run loop is the only clock reader.
//! Owned by `Daemon`, never by a pair, so a pass cannot reschedule anything.
//!
//! `pop` order: the explicit FIFO first (`syncnow`, `resync`, `reset-index`, `keep` and `apply`
//! arrive as [`JobKind::Sync`], `plan` as [`JobKind::Plan`]), not gated on time; otherwise the due
//! pair with the earliest deadline, exact ties broken by a cursor rotating over config order. An
//! explicit `Sync` for a pair whose sweep is overdue pops as that sweep (one full pass, #193).
//!
//! **No spin, by construction**: a timer or sweep pop re-arms provisionally before it
//! returns, so a caller that never reaches its own `rearm` cannot leave a pair due; `pop` and
//! `next_wake` read one deadline ([`DueQueue::deadline`]), so `pop(now) == None` implies
//! `next_wake() > now`; and a zero cadence, which would make every provisional re-arm "due now",
//! is refused by assertion.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// What a popped job asks the daemon to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobKind {
    /// A reconcile pass, or the arm that stands in for one on a paused or unavailable pair.
    Sync,
    /// A plan-only pass (#100). Inert, so it never re-arms the pair's timer.
    Plan,
}

/// Why a job was popped. Only [`Cause::Sweep`] changes what the daemon does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cause {
    /// Asked for over the control socket.
    Explicit,
    /// The pair's cadence elapsed.
    Timer,
    /// The pair's scheduled full sweep (#193) came due — alone, or beside an explicit `Sync` for the
    /// same pair, which it absorbs. Cleared on pop; the daemon re-arms it for this cause only.
    Sweep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Job {
    /// Config index: the same index as `Daemon::pairs`, `ControlShared::pairs` and
    /// `ControlPlane::pairs`.
    pub(crate) pair: usize,
    pub(crate) kind: JobKind,
    pub(crate) cause: Cause,
}

/// Bound for a deadline that `Instant` arithmetic cannot represent: a configured cadence of
/// `u64::MAX` seconds parks the pair rather than panicking on overflow. Thirty years, as tokio's
/// own far-future sleep.
const FAR_FUTURE: Duration = Duration::from_secs(86_400 * 365 * 30);

#[derive(Debug)]
pub(crate) struct DueQueue {
    /// Per pair: when its timer next fires.
    next_due: Vec<Instant>,
    /// Per pair. Set by the daemon (`set_cadence`), never computed here: the rule
    /// (`Daemon::cadence_for`) reads the event session, which this module does not know about.
    cadence: Vec<Duration>,
    /// Per pair: the next scheduled full sweep (#193); `None` = no schedule, or none resolvable.
    sweep_at: Vec<Option<Instant>>,
    /// Requests from the control socket, in arrival order.
    explicit: VecDeque<(usize, JobKind)>,
    /// Where the next exact tie starts looking, over config order.
    cursor: usize,
}

impl DueQueue {
    /// Every pair due `now` (boot runs every pair's first pass through the queue, in config order).
    pub(crate) fn new(cadences: Vec<Duration>, now: Instant) -> Self {
        assert!(!cadences.is_empty(), "a daemon runs at least one pair");
        for cadence in &cadences {
            assert_nonzero(*cadence);
        }
        let pairs = cadences.len();
        Self {
            next_due: vec![now; pairs],
            cadence: cadences,
            sweep_at: vec![None; pairs],
            explicit: VecDeque::new(),
            cursor: 0,
        }
    }

    pub(crate) fn set_cadence(&mut self, pair: usize, cadence: Duration) {
        assert_nonzero(cadence);
        self.cadence[pair] = cadence;
    }

    /// Queues an explicit job. A no-op while the same `(pair, kind)` is still **queued**: a request
    /// made while that pair's pass is running queues one more, which is what keeps the
    /// `watch_syncnow` arithmetic exact per pair ("already in progress" → `seq + 2`).
    pub(crate) fn request(&mut self, pair: usize, kind: JobKind) {
        if !self.explicit.contains(&(pair, kind)) {
            self.explicit.push_back((pair, kind));
        }
    }

    pub(crate) fn set_sweep(&mut self, pair: usize, at: Option<Instant>) {
        self.sweep_at[pair] = at;
    }

    /// The next job, or `None` when nothing is due at `now`.
    pub(crate) fn pop(&mut self, now: Instant) -> Option<Job> {
        if let Some((pair, kind)) = self.explicit.pop_front() {
            // An explicit `Sync` popped beside an overdue sweep IS that sweep (#193): both mean "the
            // next pass is a full one", so they are one pass. Left as `Explicit` it would leave
            // `sweep_at` overdue and the next pop would walk the tree a second time — and the daemon
            // re-arms a sweep only for `Cause::Sweep`, so the cause cannot stay `Explicit` either.
            // Never a `Plan`: that pass is inert and consumes nothing, so the sweep stays pending.
            let cause = if kind == JobKind::Sync && self.take_overdue_sweep(pair, now) {
                Cause::Sweep
            } else {
                Cause::Explicit
            };
            return Some(Job { pair, kind, cause });
        }
        let pairs = self.next_due.len();
        let mut chosen: Option<(usize, Instant)> = None;
        for offset in 0..pairs {
            let pair = (self.cursor + offset) % pairs;
            if !self.is_due(pair, now) {
                continue;
            }
            let deadline = self.deadline(pair);
            // Strictly earlier wins, so an exact tie keeps the first pair at or after the cursor.
            if chosen.is_none_or(|(_, best)| deadline < best) {
                chosen = Some((pair, deadline));
            }
        }
        let (pair, _) = chosen?;
        self.cursor = (pair + 1) % pairs;
        let cause = if self.take_overdue_sweep(pair, now) {
            Cause::Sweep
        } else {
            Cause::Timer
        };
        // Provisional: the daemon re-arms from the moment the pass ends, but an arm that never gets
        // there cannot leave this pair due and spin the loop.
        self.next_due[pair] = later(now, self.cadence[pair]);
        Some(Job {
            pair,
            kind: JobKind::Sync,
            cause,
        })
    }

    /// Clears and reports the pair's sweep when it is due at `now`. The one place a sweep is
    /// consumed, so a timer pop and an explicit pop cannot disagree about what consuming means.
    fn take_overdue_sweep(&mut self, pair: usize, now: Instant) -> bool {
        if self.sweep_at[pair].is_some_and(|at| at <= now) {
            self.sweep_at[pair] = None;
            true
        } else {
            false
        }
    }

    /// `next_due = from + cadence`. Called with the moment a pass ENDED, which is what makes the
    /// order starvation-free between timer-due pairs: a pair that just ran is due after every pair
    /// that was already overdue, whatever the two cadences are.
    pub(crate) fn rearm(&mut self, pair: usize, from: Instant) {
        self.next_due[pair] = later(from, self.cadence[pair]);
    }

    /// Moves a pair's timer earlier, never later (an event session came back).
    pub(crate) fn tighten(&mut self, pair: usize, at: Instant) {
        if at < self.next_due[pair] {
            self.next_due[pair] = at;
        }
    }

    /// When the earliest timer or sweep fires. Explicit requests are not consulted: the loop drains
    /// them (`pop` until `None`) before it sleeps on this.
    pub(crate) fn next_wake(&self) -> Instant {
        (0..self.next_due.len())
            .map(|pair| self.deadline(pair))
            .min()
            .expect("a daemon runs at least one pair")
    }

    /// THE "due" rule, shared by `pop` (through `is_due`) and `next_wake`. Two rules would let `pop`
    /// answer `None` while `next_wake` is already past, and the loop would wake for ever.
    fn deadline(&self, pair: usize) -> Instant {
        match self.sweep_at[pair] {
            Some(sweep) => sweep.min(self.next_due[pair]),
            None => self.next_due[pair],
        }
    }

    fn is_due(&self, pair: usize, now: Instant) -> bool {
        self.deadline(pair) <= now
    }

    #[cfg(test)]
    pub(crate) fn next_due(&self, pair: usize) -> Instant {
        self.next_due[pair]
    }

    #[cfg(test)]
    pub(crate) fn sweep_at(&self, pair: usize) -> Option<Instant> {
        self.sweep_at[pair]
    }

    #[cfg(test)]
    pub(crate) fn cadence(&self, pair: usize) -> Duration {
        self.cadence[pair]
    }
}

fn assert_nonzero(cadence: Duration) {
    assert!(
        cadence > Duration::ZERO,
        "a zero cadence makes every provisional re-arm due at once and spins the loop"
    );
}

fn later(at: Instant, by: Duration) -> Instant {
    at.checked_add(by).unwrap_or(at + FAR_FUTURE)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn secs(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    fn timer(pair: usize) -> Option<Job> {
        Some(Job {
            pair,
            kind: JobKind::Sync,
            cause: Cause::Timer,
        })
    }

    fn explicit(pair: usize, kind: JobKind) -> Option<Job> {
        Some(Job {
            pair,
            kind,
            cause: Cause::Explicit,
        })
    }

    #[test]
    fn boot_seeds_every_pair_due_now_and_pops_them_in_config_order() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30), secs(30), secs(30)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        assert_eq!(queue.pop(t0), timer(1));
        assert_eq!(queue.pop(t0), timer(2));
        assert_eq!(queue.pop(t0), None, "every boot entry is consumed once");
    }

    #[test]
    fn an_explicit_request_jumps_a_timer_due_pair() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30), secs(30)], t0);
        queue.request(1, JobKind::Sync);
        assert_eq!(queue.pop(t0), explicit(1, JobKind::Sync));
        assert_eq!(queue.pop(t0), timer(0));
    }

    #[test]
    fn explicit_requests_run_in_arrival_order_across_pairs() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30), secs(30), secs(30)], t0);
        queue.request(2, JobKind::Sync);
        queue.request(0, JobKind::Plan);
        queue.request(1, JobKind::Sync);
        assert_eq!(queue.pop(t0), explicit(2, JobKind::Sync));
        assert_eq!(queue.pop(t0), explicit(0, JobKind::Plan));
        assert_eq!(queue.pop(t0), explicit(1, JobKind::Sync));
    }

    #[test]
    fn a_repeated_request_for_a_queued_pair_coalesces_into_one_job() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30), secs(30)], t0);
        queue.request(1, JobKind::Sync);
        queue.request(1, JobKind::Sync);
        // A different kind for the same pair is a different job.
        queue.request(1, JobKind::Plan);
        assert_eq!(queue.pop(t0), explicit(1, JobKind::Sync));
        assert_eq!(queue.pop(t0), explicit(1, JobKind::Plan));
        assert_eq!(queue.pop(t0), timer(0), "the duplicate was dropped");
    }

    #[test]
    fn a_request_after_the_pair_was_popped_queues_one_more_job() {
        // The "already in progress" half of the client arithmetic: a request made while pair 1's
        // pass runs must produce one more pass, or `watch_syncnow` waits for `seq + 2` for ever.
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30), secs(30)], t0);
        queue.request(1, JobKind::Sync);
        assert_eq!(queue.pop(t0), explicit(1, JobKind::Sync));
        queue.request(1, JobKind::Sync);
        assert_eq!(queue.pop(t0), explicit(1, JobKind::Sync));
    }

    #[test]
    fn a_popped_timer_job_is_not_due_again_until_its_cadence() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        // The caller never re-armed (a skipped arm, a forgotten call): still not due again.
        assert_eq!(queue.pop(t0), None);
        assert_eq!(queue.pop(t0 + secs(29)), None);
        assert_eq!(queue.pop(t0 + secs(30)), timer(0));
    }

    #[test]
    fn after_pop_returns_none_next_wake_is_in_the_future() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30), secs(300)], t0);
        queue.set_sweep(1, Some(t0 + secs(10)));
        let mut now = t0;
        for _ in 0..50 {
            while queue.pop(now).is_some() {}
            let wake = queue.next_wake();
            assert!(
                wake > now,
                "pop answered None at {now:?} but next_wake is {wake:?}"
            );
            now = wake;
        }
    }

    #[test]
    fn a_thirty_second_pair_never_starves_a_three_hundred_second_pair() {
        // Pair 0's passes (40s) outlast its own 30s cadence, so it is overdue the moment each one
        // ends. Re-armed from the pass END it still yields to pair 1, which therefore never waits
        // more than the one pair-0 pass already running when it fell due.
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30), secs(300)], t0);
        let mut now = t0;
        let mut runs = [0usize; 2];
        let mut worst_wait = Duration::ZERO;
        while now < t0 + secs(3600) {
            let due_1 = queue.next_due(1);
            match queue.pop(now) {
                Some(job) => {
                    runs[job.pair] += 1;
                    if job.pair == 1 {
                        worst_wait = worst_wait.max(now.saturating_duration_since(due_1));
                    }
                    now += if job.pair == 0 { secs(40) } else { secs(5) };
                    queue.rearm(job.pair, now);
                }
                None => now = queue.next_wake(),
            }
        }
        assert!(
            worst_wait <= secs(40),
            "pair 1 waited {worst_wait:?} past its deadline, longer than the one pair-0 pass \
             it can land behind"
        );
        assert!(runs[1] >= 10, "{runs:?}");
        assert!(runs[0] > runs[1], "{runs:?}");
    }

    #[test]
    fn a_long_pass_delays_but_does_not_starve_the_pair_due_during_it() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30), secs(600)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        assert_eq!(queue.pop(t0), timer(1));
        queue.rearm(0, t0);
        queue.rearm(1, t0);
        // Pair 0 is due at t0 + 30s and its pass takes 30 minutes; pair 1 falls due during it, at
        // t0 + 600s — later than pair 0's own deadline would be if it were re-armed from its start.
        assert_eq!(queue.pop(t0 + secs(30)), timer(0));
        let end = t0 + secs(30) + secs(1800);
        queue.rearm(0, end);
        assert_eq!(
            queue.pop(end),
            timer(1),
            "the pair that fell due mid-pass runs before the long pair's next pass"
        );
    }

    #[test]
    fn one_explicit_request_delays_a_timer_due_pair_by_exactly_one_pass() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30), secs(30)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        assert_eq!(queue.pop(t0), timer(1));
        queue.rearm(0, t0);
        queue.rearm(1, t0);
        // Both timer-due at t0 + 30s, and pair 0 is also asked for explicitly at that moment.
        let now = t0 + secs(30);
        queue.request(0, JobKind::Sync);
        assert_eq!(queue.pop(now), explicit(0, JobKind::Sync));
        // The daemon re-arms after ANY sync job, so the explicit pass consumes pair 0's timer too.
        let end = now + SECOND;
        queue.rearm(0, end);
        assert_eq!(
            queue.pop(end),
            timer(1),
            "the explicit job is consumed, not re-queued behind itself"
        );
        assert_eq!(
            queue.pop(end),
            None,
            "one explicit pass, one delay, no extra pass"
        );
    }

    #[test]
    fn a_skipped_pair_advances_its_due_time_and_fires_no_backlog() {
        // A paused (or unavailable) pair is re-armed from `now` each time it is skipped, so resuming
        // after an hour costs one pass at its next cadence, not 120 queued ones.
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30)], t0);
        let mut now = t0;
        for _ in 0..120 {
            assert_eq!(queue.pop(now), timer(0));
            queue.rearm(0, now);
            now += secs(30);
        }
        // Resume an hour later: exactly one job is due, and the next one is a full cadence away.
        now += secs(3600);
        assert_eq!(queue.pop(now), timer(0));
        queue.rearm(0, now);
        assert_eq!(queue.pop(now), None);
        assert_eq!(queue.next_wake(), now + secs(30));
    }

    #[test]
    fn a_plan_job_does_not_rearm_the_timer() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        queue.rearm(0, t0);
        queue.request(0, JobKind::Plan);
        assert_eq!(queue.pop(t0 + secs(10)), explicit(0, JobKind::Plan));
        // The daemon does not call `rearm` for a plan: the timer stays where the last sync put it.
        assert_eq!(queue.next_due(0), t0 + secs(30));
        assert_eq!(queue.pop(t0 + secs(30)), timer(0));
    }

    #[test]
    fn a_due_sweep_pops_as_a_sweep_and_clears_until_re_armed() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(3600)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        queue.rearm(0, t0);
        queue.set_sweep(0, Some(t0 + secs(60)));
        assert_eq!(queue.pop(t0 + secs(59)), None);
        assert_eq!(
            queue.pop(t0 + secs(60)),
            Some(Job {
                pair: 0,
                kind: JobKind::Sync,
                cause: Cause::Sweep,
            })
        );
        queue.rearm(0, t0 + secs(61));
        assert_eq!(
            queue.pop(t0 + secs(120)),
            None,
            "a popped sweep is cleared; only the daemon re-arms it"
        );
        assert_eq!(queue.next_wake(), t0 + secs(61) + secs(3600));
    }

    fn sweep(pair: usize) -> Option<Job> {
        Some(Job {
            pair,
            kind: JobKind::Sync,
            cause: Cause::Sweep,
        })
    }

    #[test]
    fn an_explicit_sync_popped_beside_an_overdue_sweep_is_that_sweep() {
        // #193: a `resync` and a due sweep are one full pass. Popped as `Explicit`, the request would
        // walk the tree and leave `sweep_at` overdue, so the very next pop would walk it again.
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(3600)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        queue.rearm(0, t0);
        queue.set_sweep(0, Some(t0 + secs(60)));
        queue.request(0, JobKind::Sync);
        assert_eq!(
            queue.pop(t0 + secs(61)),
            sweep(0),
            "`Cause::Sweep`, not `Explicit`: the daemon re-arms the sweep only for that cause"
        );
        assert_eq!(queue.sweep_at(0), None, "consumed by the one pop");
        assert_eq!(
            queue.pop(t0 + secs(61)),
            None,
            "no second full walk follows"
        );
    }

    #[test]
    fn an_explicit_sync_before_the_sweep_is_due_leaves_the_sweep_armed() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(3600)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        queue.rearm(0, t0);
        queue.set_sweep(0, Some(t0 + secs(60)));
        queue.request(0, JobKind::Sync);
        assert_eq!(queue.pop(t0 + secs(59)), explicit(0, JobKind::Sync));
        assert_eq!(queue.sweep_at(0), Some(t0 + secs(60)));
    }

    #[test]
    fn an_explicit_plan_never_consumes_an_overdue_sweep() {
        // A plan-only pass observes and consumes nothing. Folding the sweep into it would clear
        // `sweep_at` for a pass that walks the remote and changes no latch, so the sweep would be
        // lost: `run_job` re-arms on a `Sync` sweep only.
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(3600)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        queue.rearm(0, t0);
        queue.set_sweep(0, Some(t0 + secs(60)));
        queue.request(0, JobKind::Plan);
        assert_eq!(queue.pop(t0 + secs(61)), explicit(0, JobKind::Plan));
        assert_eq!(queue.sweep_at(0), Some(t0 + secs(60)), "still pending");
        assert_eq!(
            queue.pop(t0 + secs(61)),
            sweep(0),
            "and it fires on the next pop"
        );
    }

    #[test]
    fn an_explicit_sync_consumes_only_its_own_pairs_sweep() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(3600), secs(3600)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        assert_eq!(queue.pop(t0), timer(1));
        queue.rearm(0, t0);
        queue.rearm(1, t0);
        queue.set_sweep(0, Some(t0 + secs(60)));
        queue.set_sweep(1, Some(t0 + secs(60)));
        queue.request(1, JobKind::Sync);
        assert_eq!(queue.pop(t0 + secs(61)), sweep(1));
        assert_eq!(
            queue.sweep_at(0),
            Some(t0 + secs(60)),
            "pair 0's is untouched"
        );
        assert_eq!(queue.pop(t0 + secs(61)), sweep(0));
    }

    #[test]
    fn next_wake_is_the_earliest_deadline_including_sweeps() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(300), secs(300)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        assert_eq!(queue.pop(t0), timer(1));
        queue.rearm(0, t0);
        queue.rearm(1, t0);
        assert_eq!(queue.next_wake(), t0 + secs(300));
        queue.set_sweep(1, Some(t0 + secs(42)));
        assert_eq!(queue.next_wake(), t0 + secs(42));
    }

    #[test]
    fn tighten_only_moves_a_deadline_earlier() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(300)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        queue.rearm(0, t0);
        queue.tighten(0, t0 + secs(400));
        assert_eq!(queue.next_due(0), t0 + secs(300));
        queue.tighten(0, t0 + secs(30));
        assert_eq!(queue.next_due(0), t0 + secs(30));
    }

    #[test]
    fn an_exact_tie_rotates_rather_than_always_favouring_the_first_pair() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![secs(30), secs(30)], t0);
        assert_eq!(queue.pop(t0), timer(0));
        // Pair 0 falls due again at the very instant pair 1 has been due since: an exact tie. The
        // pair that just ran must not win it — the cursor sits after it.
        queue.tighten(0, t0);
        assert_eq!(queue.pop(t0), timer(1));
        assert_eq!(queue.pop(t0), timer(0));
    }

    #[test]
    #[should_panic(expected = "zero cadence")]
    fn a_zero_cadence_is_refused() {
        let _ = DueQueue::new(vec![Duration::ZERO], Instant::now());
    }

    #[test]
    fn an_unrepresentable_deadline_parks_rather_than_panicking() {
        let t0 = Instant::now();
        let mut queue = DueQueue::new(vec![Duration::MAX], t0);
        assert_eq!(queue.pop(t0), timer(0));
        assert!(queue.next_wake() > t0 + secs(86_400 * 365));
        assert_eq!(queue.cadence(0), Duration::MAX);
    }
}
