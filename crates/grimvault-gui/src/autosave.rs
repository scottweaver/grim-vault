//! When to write: the autosave clock as a pure state machine over an
//! injected `now`. Edits are written once they have been quiet for
//! [`QUIET`] with no gesture in flight; a failed flush retries after
//! [`RETRY`]; a suspended gate (an unresolved external change) stops
//! the clock entirely until the user decides.

use std::time::{Duration, Instant};

/// Quiet period between the last edit and its write.
pub const QUIET: Duration = Duration::from_millis(600);
/// Backoff before a failed write is retried.
pub const RETRY: Duration = Duration::from_secs(5);

/// Whether anything needs writing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    Nothing,
    Edits,
}

/// Whether the user is mid-gesture (a drag, a held pointer button).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Busy,
}

/// Whether writing is allowed at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Open,
    /// An external change is unresolved; nothing is written until the
    /// user chooses.
    Suspended,
}

/// What the clock says this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing to do.
    Idle,
    /// Look again after this long.
    Wait(Duration),
    /// Write the dirty documents now.
    Flush,
}

/// The state shown in the status bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutosaveState {
    Saved,
    Unsaved,
    /// A write is due or being retried.
    Saving,
    Suspended,
}

impl std::fmt::Display for AutosaveState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Saved => "saved",
            Self::Unsaved => "unsaved",
            Self::Saving => "saving…",
            Self::Suspended => "suspended: external change",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer {
    Idle,
    Quiet { deadline: Instant },
    Retry { at: Instant },
}

/// The clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Autosave {
    timer: Timer,
}

impl Default for Autosave {
    fn default() -> Self {
        Self { timer: Timer::Idle }
    }
}

impl Autosave {
    /// One tick.
    pub fn observe(
        &mut self,
        pending: Pending,
        activity: Activity,
        gate: Gate,
        now: Instant,
    ) -> Verdict {
        match (gate, pending) {
            (Gate::Suspended, _) | (_, Pending::Nothing) => {
                self.timer = Timer::Idle;
                return Verdict::Idle;
            }
            (Gate::Open, Pending::Edits) => {}
        }
        let due = match self.timer {
            Timer::Idle => {
                self.timer = Timer::Quiet {
                    deadline: now + QUIET,
                };
                return Verdict::Wait(QUIET);
            }
            Timer::Quiet { deadline } => deadline,
            Timer::Retry { at } => at,
        };
        let due = match activity {
            Activity::Busy => {
                let pushed = now + QUIET;
                self.timer = match self.timer {
                    Timer::Retry { at } if at > pushed => Timer::Retry { at },
                    Timer::Idle | Timer::Quiet { .. } | Timer::Retry { .. } => {
                        Timer::Quiet { deadline: pushed }
                    }
                };
                pushed.max(due)
            }
            Activity::Idle => due,
        };
        if now < due {
            Verdict::Wait(due - now)
        } else {
            Verdict::Flush
        }
    }

    /// The flush landed; the clock idles until the next edit.
    pub fn flushed(&mut self) {
        self.timer = Timer::Idle;
    }

    /// The flush failed; retry after [`RETRY`], returned for the
    /// caller's repaint request.
    pub fn flush_failed(&mut self, now: Instant) -> Duration {
        self.timer = Timer::Retry { at: now + RETRY };
        RETRY
    }

    /// The status-bar state for this tick's inputs.
    #[must_use]
    pub fn state(&self, pending: Pending, gate: Gate) -> AutosaveState {
        match (gate, pending, self.timer) {
            (_, Pending::Nothing, _) => AutosaveState::Saved,
            (Gate::Suspended, Pending::Edits, _) => AutosaveState::Suspended,
            (Gate::Open, Pending::Edits, Timer::Retry { .. }) => AutosaveState::Saving,
            (Gate::Open, Pending::Edits, Timer::Idle | Timer::Quiet { .. }) => {
                AutosaveState::Unsaved
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> (Autosave, Instant) {
        (Autosave::default(), Instant::now())
    }

    #[test]
    fn nothing_pending_is_idle_and_reads_saved() {
        let (mut clock, now) = clock();
        assert_eq!(
            clock.observe(Pending::Nothing, Activity::Idle, Gate::Open, now),
            Verdict::Idle
        );
        assert_eq!(
            clock.state(Pending::Nothing, Gate::Open),
            AutosaveState::Saved
        );
    }

    #[test]
    fn edits_flush_after_the_quiet_period() {
        let (mut clock, start) = clock();
        assert_eq!(
            clock.observe(Pending::Edits, Activity::Idle, Gate::Open, start),
            Verdict::Wait(QUIET)
        );
        assert_eq!(
            clock.state(Pending::Edits, Gate::Open),
            AutosaveState::Unsaved
        );
        assert_eq!(
            clock.observe(
                Pending::Edits,
                Activity::Idle,
                Gate::Open,
                start + QUIET / 2
            ),
            Verdict::Wait(QUIET / 2)
        );
        assert_eq!(
            clock.observe(Pending::Edits, Activity::Idle, Gate::Open, start + QUIET),
            Verdict::Flush
        );
        clock.flushed();
        assert_eq!(
            clock.observe(Pending::Nothing, Activity::Idle, Gate::Open, start + QUIET),
            Verdict::Idle
        );
    }

    #[test]
    fn a_gesture_in_flight_pushes_the_deadline() {
        let (mut clock, start) = clock();
        clock.observe(Pending::Edits, Activity::Idle, Gate::Open, start);
        assert_eq!(
            clock.observe(Pending::Edits, Activity::Busy, Gate::Open, start + QUIET),
            Verdict::Wait(QUIET)
        );
        let almost = (QUIET * 2).checked_sub(Duration::from_millis(1)).unwrap();
        assert_eq!(
            clock.observe(Pending::Edits, Activity::Idle, Gate::Open, start + almost),
            Verdict::Wait(Duration::from_millis(1))
        );
        assert_eq!(
            clock.observe(
                Pending::Edits,
                Activity::Idle,
                Gate::Open,
                start + QUIET * 2
            ),
            Verdict::Flush
        );
    }

    #[test]
    fn a_failed_flush_retries_later_and_reads_saving() {
        let (mut clock, start) = clock();
        clock.observe(Pending::Edits, Activity::Idle, Gate::Open, start);
        assert_eq!(
            clock.observe(Pending::Edits, Activity::Idle, Gate::Open, start + QUIET),
            Verdict::Flush
        );
        assert_eq!(clock.flush_failed(start + QUIET), RETRY);
        assert_eq!(
            clock.state(Pending::Edits, Gate::Open),
            AutosaveState::Saving
        );
        assert_eq!(
            clock.observe(
                Pending::Edits,
                Activity::Idle,
                Gate::Open,
                start + QUIET + RETRY / 2
            ),
            Verdict::Wait(RETRY / 2)
        );
        assert_eq!(
            clock.observe(
                Pending::Edits,
                Activity::Idle,
                Gate::Open,
                start + QUIET + RETRY
            ),
            Verdict::Flush
        );
    }

    #[test]
    fn a_gesture_never_shortens_a_retry_backoff() {
        let (mut clock, start) = clock();
        clock.flush_failed(start);
        assert_eq!(
            clock.observe(Pending::Edits, Activity::Busy, Gate::Open, start),
            Verdict::Wait(RETRY)
        );
        assert_eq!(
            clock.observe(Pending::Edits, Activity::Idle, Gate::Open, start + RETRY),
            Verdict::Flush
        );
    }

    #[test]
    fn a_suspended_gate_stops_the_clock_and_reads_suspended() {
        let (mut clock, start) = clock();
        clock.observe(Pending::Edits, Activity::Idle, Gate::Open, start);
        assert_eq!(
            clock.observe(
                Pending::Edits,
                Activity::Idle,
                Gate::Suspended,
                start + QUIET
            ),
            Verdict::Idle
        );
        assert_eq!(
            clock.state(Pending::Edits, Gate::Suspended),
            AutosaveState::Suspended
        );
        assert_eq!(
            clock.observe(Pending::Edits, Activity::Idle, Gate::Open, start + QUIET),
            Verdict::Wait(QUIET)
        );
    }
}
