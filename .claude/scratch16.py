def patch(path, pairs):
    s = open(path).read()
    for old, new in pairs:
        assert s.count(old) == 1, (path, s.count(old), old[:110])
        s = s.replace(old, new, 1)
    open(path, "w").write(s)


# ---- 1. `append(null)` prints the word, and the line could never appear after the first render ---
patch(
    "gui/src/js/screens/plan.js",
    [
        (
            """  const progressText = checkingProgressText(progress);
  const progressLine =
    progressText == null
      ? null
      : fid(el("div", { class: "pl-checking-progress" }, progressText), "checkingProgress");
  if (progressLine) seamMask(progressLine, { pad: 14, padY: 2 });""",
            """  const progressText = checkingProgressText(progress);""",
        ),
        (
            "  body.append(mark, title, sub, progressLine, stop);\n  return [body];",
            """  body.append(mark, title, sub, stop);
  // Appended CONDITIONALLY, never as a `null` child: `Element.append(null)` inserts the literal
  // string "null", which no gate here can see (every frame's fixture carries the progress, so the
  // absent case is undrawn).
  if (progressText != null) insertCheckingProgress(body, progressText);
  return [body];""",
        ),
        (
            """/**
 * Move the progress line's numbers without rebuilding the body (#209).
 *
 * `signatureOf` returns a constant for the checking body on purpose — folding anything into it
 * would restart the mark's two CSS animations from 0% on every poll, which is the failure
 * `updateHexagon` exists to prevent. So a climbing count has to be patched in place, exactly as the
 * bar's `Checked N ago` is.
 *
 * Only the TEXT is patched, never the node's presence: a line that appeared or vanished mid-pass
 * would change the block's height under a running animation. The first render decides whether the
 * line exists, and the pass has always reached the local scan by the time a second poll lands.
 */
function patchCheckingProgress(v) {
  if (v.body !== "checking") return;
  const node = view?.nodes?.[0]?.querySelector?.(".pl-checking-progress");
  if (!node) return;
  const text = checkingProgressText(v.progress);
  if (text != null && node.textContent !== text) node.textContent = text;
}""",
            """/** The progress line, built and masked in one place so the first render and the patch agree. */
function insertCheckingProgress(body, text) {
  const node = fid(el("div", { class: "pl-checking-progress" }, text), "checkingProgress");
  seamMask(node, { pad: 14, padY: 2 });
  // Before `Stop`, which is the frame's order and the order `.pl-stop`'s 22px gap is measured from.
  body.insertBefore(node, body.querySelector(".pl-stop"));
  return node;
}

/**
 * Add, move or drop the progress line without rebuilding the body (#209).
 *
 * `signatureOf` returns a constant for the checking body on purpose — folding anything into it
 * would restart the mark's two CSS animations from 0% on every poll, which is the failure
 * `updateHexagon` exists to prevent. So the line has to be maintained in place, exactly as the
 * bar's `Checked N ago` is.
 *
 * **Adding it here is not an optimisation, it is the only way it is ever drawn.** The screen renders
 * the checking body the instant the rehearsal is *requested* — before the daemon has acked it, let
 * alone started scanning — so the first render never has a number and a patch that could only
 * update existing text would leave the line permanently absent. Inserting a sibling is safe where
 * `replaceChildren` is not: the mark's own element is untouched, so its animations do not restart.
 */
function patchCheckingProgress(v) {
  if (v.body !== "checking") return;
  const body = view?.nodes?.[0];
  if (!body?.querySelector) return;
  const text = checkingProgressText(v.progress);
  const node = body.querySelector(".pl-checking-progress");
  if (text == null) {
    node?.remove();
  } else if (!node) {
    insertCheckingProgress(body, text);
  } else if (node.textContent !== text) {
    node.textContent = text;
  }
}""",
        ),
    ],
)


# ---- 2 + 3. the retry-limit comment, and never falling back past a daemon that answered ----------
patch(
    "gui/src-tauri/src/commands.rs",
    [
        (
            """/// Consecutive transport failures that end a plan wait, as `watch_syncnow`'s bail-out. Fewer than
/// the CLI's five because the GUI re-fires on the next `Check again`.
const PLAN_POLL_ERROR_LIMIT: u32 = 5;""",
            """/// Consecutive transport failures that end a plan wait — `watch_syncnow`'s bail-out, at the same
/// count the CLI uses, because it is answering the same question: a daemon that has stopped
/// replying five polls running is not a pass that is still working.
const PLAN_POLL_ERROR_LIMIT: u32 = 5;""",
        ),
        (
            """    let ack = ipc::command(socket, ControlCommand::Plan, ipc::DEFAULT_TIMEOUT)
        .map_err(|_| DaemonPlanFailure::Unavailable)?;
    let target = match ack.plan {
        Some(PlanOutcome::Scheduled { plan_seq }) => plan_seq,
        Some(PlanOutcome::Paused) => {
            return Err(DaemonPlanFailure::Reported(
                "Syncing is paused, so nothing was worked out. Resume syncing and check again."
                    .to_owned(),
            ));
        }
        // A daemon too old to know the verb rejects the command; so does one that is shutting
        // down. Both mean "ask the child instead".
        _ => return Err(DaemonPlanFailure::Unavailable),
    };""",
            """    let ack = match ipc::command(socket, ControlCommand::Plan, ipc::DEFAULT_TIMEOUT) {
        Ok(ack) => ack,
        // The request did not complete. That is EITHER no daemon (onboarding, the case the child
        // exists for) OR a daemon too old to parse the verb — which drops the connection without
        // replying, so at this layer the two look identical. Ask `status`, which every daemon that
        // has ever existed answers, and let the answer decide.
        Err(error) => return Err(classify_unreachable_plan(socket, error)),
    };
    let target = match ack.plan {
        Some(PlanOutcome::Scheduled { plan_seq }) => plan_seq,
        Some(PlanOutcome::Paused) => {
            return Err(DaemonPlanFailure::Reported(
                "Syncing is paused, so nothing was worked out. Resume syncing and check again."
                    .to_owned(),
            ));
        }
        // A daemon ANSWERED and did not schedule a plan — an outcome this build does not know, or
        // none at all. Reported, never fallen back from: spawning `proton-syncd --dry-run` beside a
        // live daemon is two `proton-drive` clients against the CLI's shared, not-concurrency-safe
        // store (#23/#317), which is the hazard this verb exists to retire.
        other => {
            return Err(DaemonPlanFailure::Reported(format!(
                "the sync daemon did not work out a plan ({}). It may be older than this app — \
                 restart it from Settings and try again.",
                describe_plan_outcome(other.as_ref())
            )));
        }
    };""",
        ),
        (
            """/// `apply <token>` (#100), and with `skip_destructive` the Plan screen's
/// `Run it without the deletion` (#192).""",
            """/// Which failure a `plan` request that never completed really was.
///
/// `Unavailable` — and therefore the child `--dry-run` — **only** when nothing answers the socket at
/// all. A daemon that answers `status` but not `plan` is one this app is newer than, and the honest
/// answer there is to say so: falling back would run a second `proton-drive` client beside a live
/// daemon (#317), which is exactly what the verb removed.
fn classify_unreachable_plan(
    socket: &std::path::Path,
    error: ipc::IpcError,
) -> DaemonPlanFailure {
    match ipc::command(socket, ControlCommand::Status, ipc::DEFAULT_TIMEOUT) {
        Ok(_) => DaemonPlanFailure::Reported(format!(
            "the sync daemon is running but could not work out a plan ({error}). It is probably \
             older than this app — restart it from Settings and try again."
        )),
        Err(_) => DaemonPlanFailure::Unavailable,
    }
}

/// A plan outcome as a short phrase, for the one message that has to name one. Never used to
/// *decide* anything — that is what the typed outcome is for (#103).
fn describe_plan_outcome(outcome: Option<&PlanOutcome>) -> &'static str {
    match outcome {
        None => "it sent no plan at all",
        Some(PlanOutcome::Absent) => "it has no plan",
        Some(PlanOutcome::Computing { .. }) => "it is still working one out",
        Some(PlanOutcome::Failed { .. }) => "it failed",
        Some(PlanOutcome::Unknown) => "it answered in a way this app does not understand",
        Some(PlanOutcome::Scheduled { .. } | PlanOutcome::Computed(_) | PlanOutcome::Paused) => {
            "unexpectedly"
        }
    }
}

/// `apply <token>` (#100), and with `skip_destructive` the Plan screen's
/// `Run it without the deletion` (#192).""",
        ),
    ],
)
