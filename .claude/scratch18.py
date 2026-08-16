def patch(path, pairs):
    s = open(path).read()
    for old, new in pairs:
        assert s.count(old) == 1, (path, s.count(old), old[:110])
        s = s.replace(old, new, 1)
    open(path, "w").write(s)


patch(
    "gui/src-tauri/src/commands.rs",
    [
        (
            r'''        other => {
            return Err(DaemonPlanFailure::Reported(format!(
                "the sync daemon did not work out a plan — {}. It may be older than this app;                  restart it from Settings and try again.",
                describe_plan_outcome(other.as_ref())
            )));
        }''',
            r"""        other => return Err(DaemonPlanFailure::Reported(unexpected_plan_ack(other.as_ref()))),""",
        ),
        (
            r'''            "the sync daemon is running but could not work out a plan ({error}). It is probably              older than this app; restart it from Settings and try again."''',
            r'''            "the sync daemon is running but could not work out a plan ({error}). It is \
             probably older than this app; restart it from Settings and try again."''',
        ),
        (
            r'''/// A plan outcome as a short phrase, for the one message that has to name one. Never used to
/// *decide* anything — that is what the typed outcome is for (#103).
fn describe_plan_outcome(outcome: Option<&PlanOutcome>) -> &'static str {
    match outcome {
        None => "it sent no plan at all",
        Some(PlanOutcome::Absent) => "it has none",
        Some(PlanOutcome::Computing { .. }) => "it is still working one out",
        Some(PlanOutcome::Failed { .. }) => "it failed",
        Some(PlanOutcome::Unknown) => "it answered in a way this app does not understand",
        Some(PlanOutcome::Scheduled { .. } | PlanOutcome::Computed(_) | PlanOutcome::Paused) => {
            "unexpectedly"
        }
    }
}''',
            r'''/// What to tell the user when the daemon answered `plan` with something other than an ack.
///
/// One sentence per outcome rather than one sentence with the outcome interpolated, because the
/// **advice** differs: "restart it, it is older than this app" is right for a reply carrying no plan
/// field or one this build cannot read, and wrong for a daemon that tried and failed. Exhaustive by
/// variant, no `_`: a new outcome has to be given its own sentence rather than inherit an arm's
/// guess. Never used to *decide* anything — that is what the typed outcome is for (#103).
fn unexpected_plan_ack(outcome: Option<&PlanOutcome>) -> &'static str {
    match outcome {
        // Both mean this app is newer than the daemon: the field is missing, or its value comes
        // from a vocabulary this build predates.
        None | Some(PlanOutcome::Unknown) => {
            "the sync daemon did not answer with a plan. It is probably older than this app — \
             restart it from Settings and try again."
        }
        Some(PlanOutcome::Failed { .. }) => {
            "the sync daemon could not work out a plan. Check again in a moment."
        }
        // Both answer a different question than the one just asked, so either here means the
        // daemon did not schedule the pass.
        Some(PlanOutcome::Absent | PlanOutcome::Computing { .. }) => {
            "the sync daemon did not start working out a plan. Check again in a moment."
        }
        // Handled by the arms above; named so the match stays exhaustive by variant.
        Some(PlanOutcome::Scheduled { .. } | PlanOutcome::Computed(_) | PlanOutcome::Paused) => {
            "the sync daemon answered unexpectedly. Check again in a moment."
        }
    }
}''',
        ),
        (
            r'''        return Err(DaemonPlanFailure::Reported(unexpected_plan_ack(other.as_ref()))),''',
            r'''        return Err(DaemonPlanFailure::Reported(
            unexpected_plan_ack(other.as_ref()).to_owned(),
        )),''',
        ),
    ],
)

# The guard the agent note asks for: assert the RENDERED value, not the escape.
patch(
    "gui/src-tauri/src/commands.rs",
    [
        (
            r"""    /// The fork behind the plan verb (#100/#209/#317), and the second condition is the subtle one:""",
            r'''    /// Every user-facing sentence this module builds, rendered rather than merely constructed.
    ///
    /// A `\`-newline continuation in a Rust literal is silently eaten when a patch script writes it
    /// from a non-raw Python string, baking the next line's indentation into the value — invisible
    /// to `cargo fmt`, to clippy, and to any test that compares a constant with itself. So this
    /// asserts the thing a user would see: no run of spaces inside a sentence.
    /// (`docs/agent-notes/python-patch-scripts-and-rust-string-continuations.md`.)
    #[test]
    fn no_message_carries_a_swallowed_line_continuation() {
        use gui_core::wire::PlanOutcome;

        for outcome in [
            None,
            Some(PlanOutcome::Unknown),
            Some(PlanOutcome::Absent),
            Some(PlanOutcome::Failed {
                plan_seq: 1,
                error: "x".to_owned(),
            }),
            Some(PlanOutcome::Computing { plan_seq: 1 }),
            Some(PlanOutcome::Paused),
        ] {
            let message = super::unexpected_plan_ack(outcome.as_ref());
            assert!(!message.contains("  "), "{message:?}");
        }
    }

    /// The fork behind the plan verb (#100/#209/#317), and the second condition is the subtle one:''',
        ),
    ],
)
