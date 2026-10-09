// The folder dialogs (#102 phase 5c-2): adding a folder and removing one. `08-settings.md`'s Folders tab.
//
// BOTH ARE DIALOGS, not the full-window first-run (decision D7): the takeover is for a machine with no
// folder, it writes the implicit single pair, and at two folders it is shut (E14). Adding a second is a
// small question asked from Settings, or from the ⋯ menu at any count, and then the same merge dialog
// first-run shows — addressed to the NEW folder.
//
// WHAT THE ADD DIALOG CAN SAY ABOUT WHAT HAPPENS NEXT, AND WHAT IT CANNOT (decision D6). Before a new
// folder is added nothing can plan it: the child `--dry-run` is forbidden beside a live daemon, and the
// daemon cannot plan a folder it does not know. So there is no rehearsal, and the dialog says exactly
// that — no preview — and prices both sides instead, which is the one thing that can be known first (a
// wrong folder is noticed by its size, not by its name). It never says what the first sync will do to a
// file, because nothing computed it.
//
// THE ENGINE'S WORDS ARE QUOTED (voice rule 4). The refusal of a name, a relative folder, an overlap and
// the survivor of an earlier index arrive as text from the commands that decided them, are drawn in
// mono where they are errors, and are not copy.
//
// IT IS REBUILT, NOT PATCHED, when its shape changes — the dialog layer replaces the surface's children
// when `signature` moves (`app.js`) — so the fields' VALUES are never in the signature (a keystroke would
// rebuild the field under its caret); what is in it is which blocks exist and what they say.

import { el } from "../ui/el.js";
import { FOLDERS, MAIN, SETTINGS } from "../ui/copy.js";
import { button, setButtonKind, textInput } from "../ui/controls.js";
import { dialogBody, dialogFoot } from "../ui/dialog.js";
import { eyebrow } from "../ui/rows.js";
import { priceOf } from "../folders.js";
import { fid } from "../fixtures/frames.js";

/** A secondary-filled button at the dialog's size: `Choose…`, `Add`. */
const smallFilled = (label, onClick, padding = "11px 15px") =>
  button({ kind: "secondaryFilled", label, onClick, padding, radius: "var(--r-10)", fontSize: "12.5px" });

// ------------------------------------------------------------------------------- the add dialog ----

/**
 * What the add dialog's shape is: every block that exists and the words in it, and none of the values
 * typed into a field. The dialog layer compares this to decide whether to rebuild.
 */
export function addFolderShape({ flow, view }) {
  return JSON.stringify([
    flow.phase,
    view.primary,
    view.primaryEnabled,
    view.nameError,
    view.refusal,
    // The name field FOLLOWS a suggestion until the person types in it, so its value is the app's and a
    // change of it has to rebuild the field; once it is theirs it is not in the shape at all.
    flow.nameTouched ? null : flow.name,
    flow.rules,
    view.checked ? [priceOf("local", flow.probes.local), priceOf("remote", flow.probes.remote)] : null,
    view.survivor?.message ?? null,
    view.warnings,
    flow.error,
    // The accounts of earlier removals the add finished: they arrive with the add's answer, mid-dialog.
    flow.settled ?? [],
    flow.ending,
    flow.reason,
  ]);
}

/** One field's label, then what goes under it. */
function field(...children) {
  return el("div", { class: "folders-field" }, children.filter(Boolean));
}

/** The two sides' price line, or nothing before the check has priced them. */
function priceLine(side, flow, view) {
  if (!view.checked) return null;
  const price = priceOf(side, flow.probes[side]);
  if (!price) return null;
  return el("div", { class: `folders-price${price.failed ? " is-failed" : ""}` }, price.text);
}

function sideBlock(index, flow, view, handlers) {
  const local = index === 0;
  const busy = view.busy || view.settled;
  const input = fid(
    textInput({
      value: local ? flow.local : flow.remote,
      mono: true,
      class: `input is-mono folders-input${local ? "" : " is-remote"}`,
      disabled: busy,
      "aria-label": local ? MAIN.sideLocal : MAIN.sideRemote,
      "data-field": local ? "folder-local" : "folder-remote",
      onInput: (event) => handlers.onField?.(local ? "local" : "remote", event.target.value),
      onKeydown: (event) => {
        if (event.key === "Enter") handlers.onPrimary?.();
      },
    }),
    "afSideInput",
    index,
  );
  const row = fid(
    el(
      "div",
      { class: "folders-side-row" },
      input,
      local
        ? fid(
            smallFilled(SETTINGS.choose, () => handlers.onChoose?.()),
            "afChoose",
          )
        : null,
    ),
    "afSideRow",
    index,
  );
  if (local && busy) row.querySelector(".btn").disabled = true;
  const price = priceLine(local ? "local" : "remote", flow, view);
  return fid(
    el(
      "div",
      { class: `folders-side${local ? "" : " is-remote"}` },
      fid(
        eyebrow({
          tone: local ? "up" : "down",
          text: local ? MAIN.sideLocal : MAIN.sideRemote,
          align: local ? "start" : "end",
        }),
        "afSideLabel",
        index,
      ),
      row,
      price ? fid(price, "afPrice", index) : null,
    ),
    "afSide",
    index,
  );
}

/** What the dialog says while something is in flight, or after it: one line, in the voice of the bar. */
function statusLine(flow, view, sentence) {
  if (!sentence) return null;
  return el(
    "div",
    // `is-cost` is the amber of a save that did not finish; a folder the service LISTS finished everything.
    {
      class: `folders-status${view.settled && flow.ending && flow.phase !== "listed" ? " is-cost" : ""}`,
      role: "status",
    },
    sentence,
  );
}

/**
 * The add dialog's body and foot. The head — title, sub-line, ✕ — is the dialog layer's (`app.js`),
 * stamped there.
 *
 * `sentence` is the line the flow wants said in the notices' place (`Saving…`, the restart's ending),
 * built by the caller from the flow's phase because it reuses `SETTINGS`' own words for a save.
 */
export function renderAddFolder({ flow, view, handlers, sentence = null }) {
  const busy = view.busy || view.settled;
  const name = fid(
    textInput({
      value: flow.name,
      mono: true,
      class: "input is-mono folders-input",
      disabled: busy,
      "aria-label": FOLDERS.add.nameLabel,
      "data-field": "folder-name",
      onInput: (event) => handlers.onField?.("name", event.target.value),
      onKeydown: (event) => {
        if (event.key === "Enter") handlers.onPrimary?.();
      },
    }),
    "afNameInput",
  );
  const nameBlock = fid(
    field(
      fid(eyebrow({ text: FOLDERS.add.nameLabel }), "afNameLabel"),
      name,
      view.nameError
        ? fid(el("div", { class: "folders-error" }, view.nameError), "afNameNote")
        : fid(
            el("div", { class: "folders-hint" }, FOLDERS.add.nameHint(flow.name.trim() || "photos")),
            "afNameNote",
          ),
    ),
    "afNameBlock",
  );

  const sides = fid(
    el(
      "div",
      { class: "folders-sides" },
      sideBlock(0, flow, view, handlers),
      sideBlock(1, flow, view, handlers),
    ),
    "afSides",
  );

  const rules = flow.rules.map((rule) =>
    el(
      "span",
      { class: "folders-rule" },
      el("span", { class: "folders-rule-pattern" }, rule),
      button({
        kind: "quiet",
        label: "✕",
        "aria-label": `Remove the rule ${rule}`,
        padding: "0 4px",
        radius: "var(--r-5)",
        fontSize: "11px",
        disabled: busy,
        onClick: () => handlers.onRemoveRule?.(rule),
      }),
    ),
  );
  const skipBlock = fid(
    field(
      fid(eyebrow({ text: FOLDERS.add.skipLabel }), "afSkipLabel"),
      fid(el("div", { class: "folders-hint" }, FOLDERS.add.skipSub), "afSkipSub"),
      rules.length ? el("div", { class: "folders-rules" }, rules) : null,
      fid(
        el(
          "div",
          { class: "folders-skip-row" },
          fid(
            textInput({
              value: flow.draft,
              placeholder: SETTINGS.addRulePlaceholder,
              mono: true,
              class: "input is-mono folders-input",
              disabled: busy,
              "aria-label": FOLDERS.add.skipLabel,
              "data-field": "folder-rule",
              onInput: (event) => handlers.onField?.("draft", event.target.value),
              onKeydown: (event) => {
                if (event.key === "Enter") handlers.onAddRule?.();
              },
            }),
            "afSkipInput",
          ),
          fid(
            smallFilled(SETTINGS.add, () => handlers.onAddRule?.(), "11px 18px"),
            "afSkipAdd",
          ),
        ),
        "afSkipRow",
      ),
    ),
    "afSkipBlock",
  );
  if (busy) skipBlock.querySelector(".folders-skip-row .btn").disabled = true;

  // The notices, in the order a person has to take them in: an index the new folder would resume (the
  // thing most likely to surprise), anything the add would warn about, the engine's refusal, the account
  // of a save that went wrong, then what the add finished besides.
  const notices = [];
  if (view.survivor) {
    notices.push(
      fid(el("div", { class: "folders-notice", role: "note" }, view.survivor.message), "afNotice"),
    );
  }
  for (const warning of view.warnings) {
    notices.push(el("div", { class: "folders-notice", role: "note" }, warning));
  }
  if (view.refusal) {
    notices.push(
      fid(el("div", { class: "folders-error is-block", role: "alert" }, view.refusal), "afRefusal"),
    );
  }
  if (flow.error) {
    notices.push(el("div", { class: "folders-error is-block", role: "alert" }, flow.error));
  }
  // What the add did besides adding: an earlier removal it finished first, in the command's own words.
  for (const line of flow.settled ?? []) {
    notices.push(el("div", { class: "folders-notice", role: "note" }, line));
  }
  const status = statusLine(flow, view, sentence);
  if (status) notices.push(status);

  const body = fid(
    dialogBody({
      padding: "0 24px",
      marginTop: "18px",
      children: [nameBlock, sides, skipBlock, ...notices],
    }),
    "afBody",
  );
  body.classList.add("folders-body");

  // THE FOOT. The D6 sentence is the foot's text on every state of the dialog that precedes the add: it is
  // the thing a person is agreeing to by pressing the button beside it. Once the folder is added it is
  // no longer a promise about the future, and the foot says nothing (the status line above speaks).
  const primaryLabel =
    view.primary === "add"
      ? FOLDERS.add.add
      : view.primary === "done"
        ? FOLDERS.remove.done
        : view.primary === "busy"
          ? (sentence ?? FOLDERS.add.checking)
          : view.checking
            ? FOLDERS.add.checking
            : FOLDERS.add.check;
  const primary = fid(
    button({
      kind: "primary",
      size: "bar",
      label: primaryLabel,
      onClick: () => handlers.onPrimary?.(),
    }),
    "afPrimary",
  );
  if (!view.primaryEnabled || view.checking) {
    setButtonKind(primary, "primaryDisabled");
    primary.disabled = true;
  }
  const cancel = view.busy
    ? null
    : fid(
        button({
          kind: "secondary",
          size: "bar",
          label: FOLDERS.add.cancel,
          onClick: () => handlers.onCancel?.(),
        }),
        "afCancel",
      );
  const foot = fid(
    dialogFoot({
      padding: "14px 24px 18px",
      marginTop: "18px",
      gap: "12px",
      align: "center",
      children: [
        view.settled || view.busy
          ? el("span", { class: "folders-spacer" })
          : fid(el("span", { class: "folders-foot-text" }, FOLDERS.add.noPreview), "afFootText"),
        view.settled ? null : cancel,
        // After a restart that did not work, the Settings bar's own retry — here too, because this is where
        // the person is when it fails.
        flow.phase === "unresolved" && flow.ending !== "not_listed"
          ? button({
              kind: "secondary",
              size: "bar",
              label: SETTINGS.restart,
              onClick: () => handlers.onRestart?.(),
            })
          : null,
        primary,
      ].filter(Boolean),
    }),
    "afFoot",
  );
  return [body, foot];
}

// ----------------------------------------------------------------------------- the remove dialog ----

/**
 * What the removal dialog's shape is. The confirmation never changes under the person; what moves it is
 * the phase and, once it has answered, the account of what happened.
 */
export function removeFolderShape({ flow, removal, account }) {
  return JSON.stringify([flow.phase, flow.error, removal.lines, account]);
}

/**
 * The removal dialog's body and foot — the confirmation (decision D8), then, in the same dialog, the
 * answer: `account` is a list of sentences built from the command's reply, the engine's own account of
 * where the history went first. The primary of the confirmation is `Cancel` — the safe choice is the loud
 * one — and `Remove folder` is the quiet one beside it.
 */
export function renderRemoveFolder({ flow, removal, account, handlers }) {
  const answered = flow.phase === "done";
  const working = flow.phase === "removing";
  const lines = answered ? account : removal.lines;
  const body = fid(
    dialogBody({
      padding: "0 24px",
      marginTop: "16px",
      children: [
        ...lines.map((sentence, index) =>
          fid(
            el("div", { class: `folders-line${answered ? " is-account" : ""}` }, sentence),
            "rfLine",
            index,
          ),
        ),
        flow.error ? el("div", { class: "folders-error is-block", role: "alert" }, flow.error) : null,
        working
          ? el("div", { class: "folders-status", role: "status" }, FOLDERS.remove.removing(flow.pair))
          : null,
      ].filter(Boolean),
    }),
    "rfBody",
  );
  body.classList.add("folders-body", "is-lines");

  const spacer = () => fid(el("span", { class: "folders-spacer" }), "rfFootSpacer");
  const children = answered
    ? [
        spacer(),
        fid(
          button({
            kind: "primary",
            size: "bar",
            label: FOLDERS.remove.done,
            onClick: () => handlers.onDone?.(),
          }),
          "rfDone",
        ),
      ]
    : [
        spacer(),
        working
          ? null
          : fid(
              button({
                kind: "primary",
                size: "bar",
                label: FOLDERS.add.cancel,
                onClick: () => handlers.onCancel?.(),
              }),
              "rfCancel",
            ),
        fid(
          button({
            kind: "secondaryFilled",
            size: "bar",
            label: FOLDERS.remove.confirm,
            disabled: working || removal.last,
            onClick: () => handlers.onConfirm?.(),
          }),
          "rfConfirm",
        ),
      ].filter(Boolean);
  const foot = fid(
    dialogFoot({
      padding: "14px 24px 18px",
      marginTop: "16px",
      gap: "10px",
      align: "center",
      children,
    }),
    "rfFoot",
  );
  return [body, foot];
}
