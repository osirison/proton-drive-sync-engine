//! The user-facing full-sweep schedule (#193, G4) — **pure**, like [`crate::sync`]'s planner.
//!
//! # What this is, and the two things it deliberately is not
//!
//! A full sweep is the O(folders) walk that compares everything top to bottom. Three things can
//! ask for one, and they are not variants of each other:
//!
//! | | Asks for a sweep when | Job |
//! | --- | --- | --- |
//! | `events_full_scan_every` | N *in-run* incremental passes have gone by | heals a long-running process |
//! | `warm_start_full_walk_every` | N *warm starts across restarts* have gone by | heals a frequently-restarted one |
//! | **`full_scan_schedule`** (here) | **a wall-clock moment arrives** | the one a person asks for |
//!
//! The first two are **self-heal safety nets with no user-facing surface**. This is a declarative
//! trigger a person sets in Settings. It does not replace, wrap, or reimplement either of them, and
//! it deliberately does not arbitrate with them: all three mean the identical thing to the daemon —
//! *the next pass is a full one* — so they compose by setting one idempotent latch. Two of them
//! firing close together is one sweep because the latch is a `bool`, not because anything decides
//! precedence. That is the whole reason there is no precedence rule to get wrong.
//!
//! **There is no catch-up.** A scheduled moment that passes while the daemon is down is skipped,
//! not replayed: [`FullScanSchedule::next_due`] is asked for a time strictly after *now*, so a
//! daemon starting at 04:00 on a `weekly sun 03:00` schedule waits for next Sunday. The honest
//! justification, rather than "it was simpler": a daemon that is up is up at the moment by
//! definition, and one that restarts often is already swept every `warm_start_full_walk_every`
//! warm starts — the case a catch-up would cover is the case the existing net already covers. No
//! frame draws a missed-run notice, and a sweep that starts itself minutes after login because of
//! a schedule the user set for 3am is a surprise, not a service.
//!
//! # Naive on purpose
//!
//! Everything here is `NaiveDateTime`. A schedule says "3am", meaning 3am where the user is, and
//! the conversion to an instant is the daemon's ([`crate::daemon`]) — one place, so the DST rules
//! live in one place too. Keeping the arithmetic naive is also what makes it testable without a
//! timezone: every case below is deterministic in CI.

use std::fmt;
use std::str::FromStr;

use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Weekday};

use crate::{AppResult, boxed_error};

/// The canonical spelling of the key, for messages.
pub const FULL_SCAN_SCHEDULE_KEY: &str = "full_scan_schedule";

/// `weekly <day> HH:MM` or `monthly day <n>, HH:MM` — **one spelling**, which is also the one the
/// Settings screen renders in its key line, so what a person reads there is what the file holds.
///
/// Round-trips: `schedule.to_string().parse()` is the same schedule, and the rendered form is what
/// a config writer writes. There is no second spelling to have a precedence rule about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullScanSchedule {
    Weekly { day: Weekday, at: NaiveTime },
    Monthly { day: u32, at: NaiveTime },
}

impl FullScanSchedule {
    /// The next moment this schedule fires, **strictly after** `after`.
    ///
    /// Strictly, so a sweep that has just run cannot immediately re-arm on the same instant, and so
    /// a daemon starting exactly at 03:00:00 on its scheduled day does not fire twice.
    ///
    /// The monthly rule is `min(day, last day of that month)`: a `day 31` schedule fires on the
    /// 30th in April and on the 28th or 29th in February. The design draws that sentence about the
    /// 15th only because its fixture chose 15 — the rule is about any day a month does not have,
    /// and clamping is the only answer that keeps "monthly" meaning twelve sweeps a year.
    pub fn next_due(self, after: NaiveDateTime) -> NaiveDateTime {
        match self {
            Self::Weekly { day, at } => {
                let ahead = (7 + day.num_days_from_monday() as i64
                    - after.date().weekday().num_days_from_monday() as i64)
                    % 7;
                let candidate = after
                    .date()
                    .checked_add_days(Days::new(ahead as u64))
                    .map(|date| date.and_time(at));
                match candidate {
                    // `ahead == 0` is today; if the time has already gone by, it is next week.
                    Some(candidate) if candidate > after => candidate,
                    _ => after
                        .date()
                        .checked_add_days(Days::new(ahead as u64 + 7))
                        .map(|date| date.and_time(at))
                        // The end of `NaiveDate`'s range (year 262143). Unreachable from a system
                        // clock, and saturating is the only answer that is not a panic in a timer.
                        .unwrap_or(NaiveDateTime::MAX),
                }
            }
            Self::Monthly { day, at } => {
                let this_month = clamped_day(after.date(), day).and_time(at);
                if this_month > after {
                    return this_month;
                }
                let next = first_of_next_month(after.date());
                clamped_day(next, day).and_time(at)
            }
        }
    }

    /// Which weekday token this schedule renders as, for the GUI's chip row. `None` for a monthly
    /// schedule, which has no weekday at all — deliberately not "Sunday", which would draw a
    /// selected chip for a value the schedule does not hold.
    pub fn weekday(self) -> Option<Weekday> {
        match self {
            Self::Weekly { day, .. } => Some(day),
            Self::Monthly { .. } => None,
        }
    }

    /// The time of day, which both variants have.
    pub fn at(self) -> NaiveTime {
        match self {
            Self::Weekly { at, .. } | Self::Monthly { at, .. } => at,
        }
    }
}

/// `day` in `date`'s month, clamped to the last day that month has.
fn clamped_day(date: NaiveDate, day: u32) -> NaiveDate {
    let last = last_day_of_month(date.year(), date.month());
    // `day` is validated 1..=31 at parse time, so the clamp is the only thing that can fail here,
    // and it cannot: `1..=last` is always a real day of a real month.
    NaiveDate::from_ymd_opt(date.year(), date.month(), day.min(last))
        .unwrap_or_else(|| unreachable!("a clamped day is a day this month has"))
}

fn first_of_next_month(date: NaiveDate) -> NaiveDate {
    let (year, month) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(NaiveDate::MAX)
}

fn last_day_of_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|first| first.pred_opt())
        .map_or(28, |last| last.day())
}

impl fmt::Display for FullScanSchedule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Weekly { day, at } => {
                write!(
                    formatter,
                    "weekly {} {}",
                    weekday_token(*day),
                    render_time(*at)
                )
            }
            Self::Monthly { day, at } => {
                write!(formatter, "monthly day {day}, {}", render_time(*at))
            }
        }
    }
}

impl FromStr for FullScanSchedule {
    type Err = Box<dyn std::error::Error + Send + Sync>;

    /// Strict. A schedule the daemon cannot read is a sweep that silently never runs, which is the
    /// worst outcome available: the safety net the user configured is the one thing they will not
    /// notice missing. So a malformed value is fatal at startup, like `log_level` — never defaulted
    /// and never ignored.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let trimmed = text.trim();
        let rest = |prefix: &str| trimmed.strip_prefix(prefix).map(str::trim_start);
        if let Some(rest) = rest("weekly ") {
            let (day, at) = rest.split_once(' ').ok_or_else(|| malformed(trimmed))?;
            return Ok(Self::Weekly {
                day: parse_weekday(day.trim()).ok_or_else(|| malformed(trimmed))?,
                at: parse_time(at.trim()).ok_or_else(|| malformed(trimmed))?,
            });
        }
        if let Some(rest) = rest("monthly day ") {
            let (day, at) = rest.split_once(',').ok_or_else(|| malformed(trimmed))?;
            let day: u32 = day.trim().parse().map_err(|_| malformed(trimmed))?;
            if !(1..=31).contains(&day) {
                return Err(malformed(trimmed));
            }
            return Ok(Self::Monthly {
                day,
                at: parse_time(at.trim()).ok_or_else(|| malformed(trimmed))?,
            });
        }
        Err(malformed(trimmed))
    }
}

fn malformed(value: &str) -> Box<dyn std::error::Error + Send + Sync> {
    boxed_error(format!(
        "{FULL_SCAN_SCHEDULE_KEY} is `{value}`, which is not a schedule this daemon can read: \
         write `weekly <day> HH:MM` (day is one of sun mon tue wed thu fri sat) or \
         `monthly day <1-31>, HH:MM` — for example `weekly sun 03:00` or `monthly day 15, 03:00`. \
         Refused rather than ignored: a schedule that does not parse is a full sweep that silently \
         never runs, and an unrun safety net is the one thing nobody notices is missing"
    ))
}

/// Parsed and rendered here rather than through `chrono`'s `%H:%M`, because the strictness is the
/// point: `%H:%M` accepts `3:00` and a trailing remainder, and two spellings of one time is the
/// round-trip ambiguity this key has no need for.
fn parse_time(text: &str) -> Option<NaiveTime> {
    let (hour, minute) = text.split_once(':')?;
    if hour.len() != 2
        || minute.len() != 2
        || !hour
            .bytes()
            .chain(minute.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    NaiveTime::from_hms_opt(hour.parse().ok()?, minute.parse().ok()?, 0)
}

fn render_time(at: NaiveTime) -> String {
    format!("{:02}:{:02}", at.hour(), at.minute())
}

/// The seven tokens, lowercase and three letters — the spelling the Settings key line draws.
pub fn weekday_token(day: Weekday) -> &'static str {
    match day {
        Weekday::Sun => "sun",
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
    }
}

fn parse_weekday(token: &str) -> Option<Weekday> {
    Some(match token {
        "sun" => Weekday::Sun,
        "mon" => Weekday::Mon,
        "tue" => Weekday::Tue,
        "wed" => Weekday::Wed,
        "thu" => Weekday::Thu,
        "fri" => Weekday::Fri,
        "sat" => Weekday::Sat,
        _ => return None,
    })
}

/// Validate a file's `full_scan_schedule` value, for the config layer's one validation site.
pub fn validate(value: &str) -> AppResult<FullScanSchedule> {
    value.parse()
}

/// How long a gap this will step across before giving up. A DST spring-forward is normally 60
/// minutes and has been 30 in some jurisdictions; three hours is well clear of both and still
/// bounded, so a timezone database this does not anticipate cannot turn into an unbounded loop
/// inside the run loop's timer arm.
const MAX_GAP_STEP_MINUTES: i64 = 180;

/// Turn a naive scheduled moment into a real instant, which is **the one place daylight saving is
/// decided** — everything in this module above is naive precisely so that this is true.
///
/// A local time is not always exactly one instant, and both of the other cases are reachable by a
/// schedule someone would plausibly write (`03:00` is inside the changeover window in much of
/// Europe):
///
/// * **Ambiguous** — the clock went back, so `02:30` happens twice. Take the **earlier**. The sweep
///   is a safety net; running it at the first opportunity is the reading that never delays it, and
///   "an hour early once a year" is invisible while "skipped" is the failure this key exists to
///   prevent.
/// * **Nonexistent** — the clock went forward, so `02:30` never happens that day. Step forward a
///   minute at a time to the first instant that does exist, which is the moment the gap ends. The
///   alternative — skipping to next week or next month — silently drops a sweep once a year for
///   users in one timezone band, which is exactly the kind of quiet omission this key must not have.
///
/// The resolver is injected rather than being `Local` directly, so the policy is tested without
/// setting `TZ`: `std::env::set_var` is `unsafe` under edition 2024 and races any concurrent
/// `getenv` (the same reason `trash.rs` keeps its redirection out of the lib binary).
pub fn resolve_local<T, F>(due: NaiveDateTime, resolve: F) -> Option<T>
where
    F: Fn(NaiveDateTime) -> chrono::LocalResult<T>,
{
    for minute in 0..=MAX_GAP_STEP_MINUTES {
        let candidate = due + chrono::TimeDelta::minutes(minute);
        match resolve(candidate) {
            chrono::LocalResult::Single(instant) => return Some(instant),
            chrono::LocalResult::Ambiguous(earlier, _) => return Some(earlier),
            chrono::LocalResult::None => continue,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").expect("test timestamp")
    }

    fn schedule(text: &str) -> FullScanSchedule {
        text.parse().expect("test schedule")
    }

    #[test]
    fn the_one_spelling_round_trips_and_is_the_one_the_key_line_draws() {
        // The rendered form is what a config writer writes AND what Settings shows in its key
        // line, so a second accepted spelling would be a value a user reads one way and the file
        // holds another. Both forms here are copied from `08-settings.md`.
        for text in ["weekly sun 03:00", "monthly day 15, 03:00"] {
            assert_eq!(schedule(text).to_string(), text);
        }
        assert_eq!(
            schedule("weekly sat 23:59"),
            FullScanSchedule::Weekly {
                day: Weekday::Sat,
                at: NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
            }
        );
        assert_eq!(
            schedule("monthly day 1, 00:00"),
            FullScanSchedule::Monthly {
                day: 1,
                at: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            }
        );
    }

    #[test]
    fn a_schedule_the_daemon_cannot_read_is_refused_rather_than_defaulted() {
        // Every one of these is a value someone could plausibly write. A default would turn each
        // into a sweep that silently never runs, which is the failure nobody notices.
        for text in [
            "",
            "weekly",
            "weekly sunday 03:00", // long form: one spelling only
            "weekly sun 3:00",     // unpadded hour
            "weekly sun 03:00:00", // seconds
            "weekly sun 24:00",    // not a time
            "weekly sun 03:60",
            "weekly xyz 03:00",
            "weekly Sun 03:00", // case: one spelling only
            "monthly day 0, 03:00",
            "monthly day 32, 03:00",
            "monthly day 15 03:00", // missing comma
            "monthly 15, 03:00",
            "daily 03:00",
            "every sunday at 3am",
        ] {
            let error = text
                .parse::<FullScanSchedule>()
                .expect_err(&format!("{text:?} is not a schedule"));
            assert!(
                error.to_string().contains(FULL_SCAN_SCHEDULE_KEY),
                "the refusal must name the key, got {error}"
            );
        }
    }

    #[test]
    fn a_weekly_schedule_fires_on_its_day_and_strictly_after_the_moment_asked_about() {
        let sunday_3am = schedule("weekly sun 03:00");
        // 2026-08-29 is a Saturday; 2026-08-30 the Sunday after it.
        assert_eq!(
            sunday_3am.next_due(at("2026-08-29 12:00:00")),
            at("2026-08-30 03:00:00")
        );
        // Earlier the same day: today still.
        assert_eq!(
            sunday_3am.next_due(at("2026-08-30 02:59:59")),
            at("2026-08-30 03:00:00")
        );
        // STRICTLY after, which is what stops a sweep re-arming on the instant it just ran and a
        // daemon starting exactly on the boundary firing twice.
        assert_eq!(
            sunday_3am.next_due(at("2026-08-30 03:00:00")),
            at("2026-09-06 03:00:00")
        );
        // Later the same day: next week, not tonight.
        assert_eq!(
            sunday_3am.next_due(at("2026-08-30 03:00:01")),
            at("2026-09-06 03:00:00")
        );
        // Every weekday resolves to its own day, across a year boundary for the wrap.
        for (token, expected) in [
            ("mon", "2026-12-28"),
            ("tue", "2026-12-29"),
            ("wed", "2026-12-30"),
            ("thu", "2026-12-31"),
            ("fri", "2027-01-01"),
            ("sat", "2027-01-02"),
            ("sun", "2026-12-27"),
        ] {
            // 2026-12-26 is a Saturday.
            assert_eq!(
                schedule(&format!("weekly {token} 03:00")).next_due(at("2026-12-26 12:00:00")),
                at(&format!("{expected} 03:00:00")),
                "weekly {token}"
            );
        }
    }

    #[test]
    fn a_monthly_day_a_month_does_not_have_is_clamped_to_that_months_last() {
        // The rule the design draws as "Months without a 15th are skipped to the last day" — worded
        // for its fixture's day, but the rule is any day a month lacks. Clamping is the only answer
        // that keeps `monthly` meaning twelve sweeps a year rather than seven.
        let day_31 = schedule("monthly day 31, 03:00");
        for (from, expected) in [
            ("2026-01-31 04:00:00", "2026-02-28 03:00:00"), // February, common year
            ("2028-01-31 04:00:00", "2028-02-29 03:00:00"), // February, leap year
            ("2026-03-31 04:00:00", "2026-04-30 03:00:00"), // 30-day month
            ("2026-04-30 04:00:00", "2026-05-31 03:00:00"), // and back to a 31-day one
            ("2026-12-31 04:00:00", "2027-01-31 03:00:00"), // year boundary
        ] {
            assert_eq!(day_31.next_due(at(from)), at(expected), "from {from}");
        }
        // Day 29 exists in a leap February and not in a common one.
        let day_29 = schedule("monthly day 29, 03:00");
        assert_eq!(
            day_29.next_due(at("2026-02-01 00:00:00")),
            at("2026-02-28 03:00:00")
        );
        assert_eq!(
            day_29.next_due(at("2028-02-01 00:00:00")),
            at("2028-02-29 03:00:00")
        );
    }

    #[test]
    fn a_monthly_schedule_fires_this_month_when_the_day_is_still_ahead() {
        let day_15 = schedule("monthly day 15, 03:00");
        assert_eq!(
            day_15.next_due(at("2026-08-01 00:00:00")),
            at("2026-08-15 03:00:00")
        );
        assert_eq!(
            day_15.next_due(at("2026-08-15 02:59:59")),
            at("2026-08-15 03:00:00")
        );
        // Strictly after, same rule as weekly.
        assert_eq!(
            day_15.next_due(at("2026-08-15 03:00:00")),
            at("2026-09-15 03:00:00")
        );
        assert_eq!(
            day_15.next_due(at("2026-08-31 23:59:59")),
            at("2026-09-15 03:00:00")
        );
    }

    #[test]
    fn a_clock_that_went_back_runs_the_sweep_at_the_earlier_of_the_two_instants() {
        // The sweep is a safety net: the earlier reading never delays it, and an hour early once a
        // year is invisible next to a skipped sweep.
        let due = at("2026-10-25 02:30:00");
        let chosen = resolve_local(due, |naive| {
            chrono::LocalResult::Ambiguous(format!("first {naive}"), format!("second {naive}"))
        });
        assert_eq!(chosen.as_deref(), Some("first 2026-10-25 02:30:00"));
    }

    #[test]
    fn a_clock_that_went_forward_runs_the_sweep_when_the_gap_ends() {
        // 02:00..03:00 does not exist on this day. A schedule set for 02:30 must not silently lose
        // its sweep for the month; it runs at the first instant that does exist.
        let gap_start = at("2027-03-28 02:00:00");
        let gap_end = at("2027-03-28 03:00:00");
        let resolver = move |naive: NaiveDateTime| {
            if naive >= gap_start && naive < gap_end {
                chrono::LocalResult::None
            } else {
                chrono::LocalResult::Single(naive)
            }
        };
        assert_eq!(
            resolve_local(at("2027-03-28 02:30:00"), resolver),
            Some(gap_end)
        );
        // A time outside the gap is untouched.
        assert_eq!(
            resolve_local(at("2027-03-28 04:00:00"), resolver),
            Some(at("2027-03-28 04:00:00"))
        );
    }

    #[test]
    fn a_gap_no_real_timezone_has_gives_up_instead_of_looping() {
        // The bound is what stops a timezone database this does not anticipate becoming an infinite
        // loop inside the run loop's timer arm. `None` there means "no sweep scheduled", which is
        // the same state a daemon with no schedule is in — inert, not stuck.
        assert_eq!(
            resolve_local(at("2026-01-01 00:00:00"), |_: NaiveDateTime| {
                chrono::LocalResult::<NaiveDateTime>::None
            }),
            None
        );
    }

    #[test]
    fn every_answer_is_in_the_future_and_lands_on_the_schedule_it_came_from() {
        // The two properties the daemon depends on, over a year of starting points and every
        // schedule shape — a targeted case can pass while an arm the loop actually takes does not.
        // A returned time in the past would make the timer fire immediately and forever; a time on
        // the wrong day would be a sweep at an hour the user did not ask for.
        let mut day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start");
        let mut checked = 0;
        while day.year() == 2026 {
            for hour in [0, 3, 12, 23] {
                let now = day.and_hms_opt(hour, 30, 0).expect("naive time");
                for token in ["sun", "mon", "tue", "wed", "thu", "fri", "sat"] {
                    let schedule = schedule(&format!("weekly {token} 03:00"));
                    let due = schedule.next_due(now);
                    assert!(due > now, "weekly {token} from {now} gave {due}");
                    assert_eq!(weekday_token(due.date().weekday()), token, "from {now}");
                    assert_eq!(due.time(), NaiveTime::from_hms_opt(3, 0, 0).unwrap());
                    // A weekly schedule can never be more than a week away.
                    assert!(due - now <= chrono::TimeDelta::days(7), "from {now}");
                    checked += 1;
                }
                for dom in [1, 15, 28, 29, 30, 31] {
                    let schedule = schedule(&format!("monthly day {dom}, 03:00"));
                    let due = schedule.next_due(now);
                    assert!(due > now, "monthly {dom} from {now} gave {due}");
                    let last = last_day_of_month(due.year(), due.month());
                    assert_eq!(due.day(), dom.min(last), "from {now} gave {due}");
                    // And never more than two months away: the clamp must not skip a month.
                    assert!(due - now <= chrono::TimeDelta::days(62), "from {now}");
                    checked += 1;
                }
            }
            day = day.succ_opt().expect("next day");
        }
        assert_eq!(checked, 365 * 4 * 13, "every day of 2026 at four hours");
    }
}
