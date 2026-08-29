// Controls (F5). Every button, input and picker the screens compose. `01-foundations.md` §1 for the
// kinds, §3 for the radii, `14-behaviour-and-state.md` for the interactive states.
//
// MEASURED out of the 51 fixtures F8 extracted — 150 drawn buttons in the dark frames, read by
// signature rather than by eye. Two things that shape the API:
//
//   · KIND AND SIZE ARE INDEPENDENT AXES. 150 buttons make 68 distinct style signatures but only 11
//     colour roles; the rest is a size ladder. Modelling them as one flat list of 68 variants is the
//     obvious wrong turn — it hides that `Pause` on the main screen and `Pause` in the compact panel
//     are one kind at two sizes, and it makes a new screen invent a 69th.
//   · THE SIZE LADDER HAS A LONG TAIL. Six sizes cover 100 of the 150; the remaining 31 combinations
//     are mostly one or two uses each, tuned per site. So `size` names the common rungs and
//     `padding`/`radius`/`fontSize` override for the tail, the same shape as F2's strokeForSize —
//     except a button's padding being 1px out is not a fidelity failure the way a stroke is, so this
//     one does not throw.
//
// THE RULE THAT OUTRANKS THE REST: maximum contrast is the primary action. In dark that is the white
// button. It is why `Keep both` and `Keep it` are the brightest thing on their screens — the SAFE
// choice is the loud one, and a screen that makes the destructive option the prominent one has
// inverted the whole design. `01-foundations.md` §1 and the F5 issue both say so; it is the one
// thing here worth failing a review over.

import { el } from "./el.js";

// ------------------------------------------------------------------------------- the kinds ----

/**
 * Eleven colour roles, measured. The six the F5 issue names, plus five the frames draw that the
 * prose does not separate: a filled secondary, a soft primary, an outlined primary (the selected
 * pill tab), and the two the notification banners use over their own translucent surface.
 *
 * `weight` is part of the kind, not the size: every primary form is 600, every secondary 400, and
 * the decision buttons are 500 — a tier the prose never mentions.
 */
const KIND = {
  /** 69 of 150. Transparent with a hairline. The default for anything that is not the main action. */
  secondary: {
    bg: null,
    border: "var(--border)",
    color: "var(--text-3)",
    weight: 400,
    bgHover: "var(--panel-raised)",
    bgActive: "var(--border)",
    borderHover: "var(--border-strong)",
  },
  /**
   * `Pause` on `2a Syncing`, `Show them`, `Done`, the `›` stepper — a secondary that sits on a panel
   * and needs to lift off it. Twelve drawn.
   *
   * ITS THREE TOKENS ARE THE `--btn-secondary-*` ROLE, not the surface/border/text tiers that happen
   * to coincide with them in dark. F5 wrote `--panel-raised`/`--border`/`--text-2`, which measures
   * correct on every dark frame and wrong on two of three values in light: `12a Syncing light` draws
   * this button `#FFFFFF` / `#D6D2CB` / `#14161A`, where `--border` is `#E6E3DE` and `--text-2` is
   * `#374151`. Nothing caught it because S10 owns light and no light frame is mapped yet — the pair
   * is only comparable because `2a Syncing`'s twin exists and F9's `sameAs` names it. §66.
   */
  secondaryFilled: {
    bg: "var(--btn-secondary-bg)",
    border: "var(--btn-secondary-border)",
    color: "var(--btn-secondary-text)",
    weight: 400,
    bgHover: "var(--panel-raised)",
    bgActive: "var(--border)",
    borderHover: "var(--border-strong)",
  },
  /**
   * `Sync now` on `2a Settled`, `Back to sync`, both of `9a Folders`' browse buttons. Four drawn.
   *
   * The same role as `secondaryFilled` with NO FILL IN DARK — and a fill in light, which is why it
   * is a kind and not a caller passing `bg: null`. `2a Settled` draws it transparent over `#0A0B0D`
   * (IMPLEMENTATION-PLAN §1.3 conflict 4 resolved it against `03-main-screen.md`'s `#101216`) while
   * `12a Settled light` draws it `#FFFFFF` over `#FAF8F5`: light is a lighter surface, so a card that
   * reads as depth there reads as noise here. No existing token is transparent in one theme and a
   * surface in the other, hence `--btn-secondary-outline-bg`.
   */
  secondaryOutlined: {
    bg: "var(--btn-secondary-outline-bg)",
    border: "var(--btn-secondary-border)",
    color: "var(--btn-secondary-text)",
    weight: 400,
    bgHover: "var(--panel-raised)",
    bgActive: "var(--border)",
    borderHover: "var(--border-strong)",
  },
  /**
   * `Pause` and `Later` in the 360px compact panel: transparent with a hairline.
   *
   * It cannot borrow `secondary`, which is the same button in dark and a different one in light —
   * the panel's quiet border measures `#E0DCD5` where `--border` is `#E6E3DE` (DEVIATIONS.md §15's
   * role-token split). F1 tokenised this role as `--btn-quiet-*` from `01-foundations.md` §1 and
   * nothing had claimed it until F6; the values were there before the kind was.
   */
  quietOutlined: {
    bg: null,
    border: "var(--btn-quiet-border)",
    color: "var(--btn-quiet-text)",
    weight: 400,
    bgHover: "var(--panel-raised)",
    bgActive: "var(--border)",
    borderHover: "var(--border-strong)",
  },
  /**
   * `Open` beside it — the second secondary fill `01-foundations.md` §1 allows, with a brighter
   * label than `secondaryFilled`'s. `--btn-secondary-bg-alt` is F1's, and its comment already says
   * where it goes: "the compact panel uses this one".
   *
   * Its fill sits ABOVE `--panel-raised` on the surface ladder (`#16181D` against `#101216`), so
   * hover and press step up from `--border` rather than to it — the one-level rule of
   * 14-behaviour-and-state.md applied from where this kind actually starts.
   */
  secondaryAlt: {
    bg: "var(--btn-secondary-bg-alt)",
    border: "var(--btn-secondary-border)",
    color: "var(--text-bright)",
    weight: 400,
    bgHover: "var(--border)",
    bgActive: "var(--border-strong)",
    borderHover: "var(--border-strong)",
  },
  /** The `⋯`: no border at all, and the label tone.
   *
   * NOT the segmented control's unselected segment, which this comment used to claim (#193). That
   * one is `--text-3` (`#99A2AE`) in `8a Settings`, a step brighter than `--text-label`'s `#626B78`,
   * and nothing could see the difference while no screen drew a segmented control. See `segment`. */
  quiet: {
    bg: null,
    border: null,
    color: "var(--text-label)",
    weight: 400,
    bgHover: "var(--panel-raised)",
    bgActive: "var(--border)",
  },
  /** A segmented control's unselected segment: no border, no background, `--text-3`. The selected
   * one is plain `primary`, which the frame draws it as exactly. */
  segment: {
    bg: null,
    border: null,
    color: "var(--text-3)",
    weight: 400,
    bgHover: "var(--panel-raised)",
    bgActive: "var(--border)",
  },

  /** THE loud one. White in dark, near-black in light. Reserve it for the safe action. */
  primary: { bg: "var(--text)", border: null, color: "var(--surface)", weight: 600 },
  /** `2A2E36` on `6D7783` — F1 tokenised both; do not reach for a disabled `opacity`. */
  primaryDisabled: {
    bg: "var(--btn-primary-disabled-bg)",
    border: null,
    color: "var(--btn-primary-disabled-text)",
    weight: 600,
    disabled: true,
  },
  /**
   * `Keep it — put it back on Proton Drive`, `Keep both files`. Primary weight, panel fill,
   * brighter border — and the loudest thing on the Deletions screen, because it is the reversible
   * one. Fifteen drawn across `4a Deletions` and `4a Armed`.
   *
   * ITS THREE TOKENS ARE A ROLE, exactly as `secondaryFilled`'s are (§66). F5 wrote
   * `--panel-raised`/`--border-strong`/`--text-bright`, which measures correct on every dark frame
   * and wrong on all three in light: `12a Deletions light` draws `#14161A` / no border / `#FAF8F5`,
   * where those tokens are `#FFFFFF` / `#E0DCD5` / `#14161A`. In light this button IS the primary
   * button, which is `12-light-theme.md`'s own rule — "maximum contrast is the primary action…so
   * `Keep both` and `Keep it` stay the loudest thing on their screens" — reading as a token
   * identity rather than as a resemblance.
   *
   * `borderStyle` because the light frame draws no border at all: 38px dark, 36px light, which is
   * the 2px a border takes out of a border-box. Same shape of finding as `primaryChoice`.
   */
  primarySoft: {
    bg: "var(--btn-primary-soft-bg)",
    border: "var(--btn-primary-soft-border)",
    borderStyle: "var(--btn-primary-soft-border-style)",
    color: "var(--btn-primary-soft-text)",
    weight: 600,
  },
  /** The selected pill tab: a primary fill that keeps its border. */
  primaryOutlined: {
    bg: "var(--text)",
    border: "var(--border-strong)",
    color: "var(--surface)",
    weight: 600,
  },

  /** Outlined crimson. A decision to make, never a thing that happens when you click. */
  decision: {
    bg: "var(--decision-btn-bg)",
    border: "var(--decision-btn-border)",
    color: "var(--decision-text)",
    weight: 500,
  },
  /**
   * `3a Conflict`'s two losing choices — a second decision tint, DEVIATIONS §8b.
   *
   * THE ELEMENT PAINTS NOTHING. Every string inside a choice button is a span carrying its own
   * colour, so the button keeps the UA's unpainted black in both themes — `--btn-unpainted`, and
   * see the token for why that is measured rather than invented. The colour a reader actually sees
   * on the label is `labelColor`; the consequence sentence beneath it is `subColor`.
   *
   * F5 wrote this kind with `color: var(--text)` and weight 400 off the frame's *label*, which is
   * neither — the label is #FF9C9C at 600, on a span. Nothing caught it because nothing rendered a
   * decisionChoice until S2: the same undrawn-code class as the nine S1 bugs and the fourteen in
   * C1–C5, and the third time the gate could not have been the thing that found it.
   */
  decisionChoice: {
    bg: "var(--decision-choice-bg)",
    border: "var(--decision-choice-border)",
    color: "var(--btn-unpainted)",
    labelColor: "var(--decision-text)",
    subColor: "var(--text-3)",
    weight: 400,
  },
  /**
   * `Keep both` — the safe choice, sitting between the two that lose something.
   *
   * NOT `primary` at a different size. Its border is the one token whose themes disagree about
   * whether a border exists at all (`--btn-primary-choice-border-style`), and like its two
   * neighbours it paints no text of its own.
   */
  primaryChoice: {
    bg: "var(--btn-primary-bg)",
    border: "var(--btn-primary-choice-border)",
    borderStyle: "var(--btn-primary-choice-border-style)",
    color: "var(--btn-unpainted)",
    labelColor: "var(--btn-primary-text)",
    subColor: "var(--btn-primary-quiet-text)",
    weight: 400,
  },

  /**
   * THE ONLY SOLID RED IN THE APP. One drawn instance: `Delete permanently` on `4a Armed`, behind a
   * typed-DELETE gate and a full-window confirmation. If a second one ever appears, something has
   * gone wrong with the design rather than with this table.
   */
  destructive: { bg: "var(--destructive)", border: null, color: "var(--btn-destructive-text)", weight: 600 },
  /**
   * The gate's `Delete` before the word is typed.
   *
   * `--destructive-btn-border` and not `--destructive-border`: .25 against .38, and F1 tokenised
   * the pair with this exact site named in its comment. F5 reached for the card's border, which is
   * the same hue one step brighter — invisible until something rendered the button next to the card
   * it was borrowing from.
   */
  destructiveDisabled: {
    bg: "var(--btn-destructive-disabled-bg)",
    border: "var(--destructive-btn-border)",
    color: "var(--btn-destructive-disabled-text)",
    weight: 600,
    disabled: true,
  },
  /**
   * The card gate's `Delete` AFTER the word matches — the one state of this screen no frame draws,
   * because `4a Deletions` draws the gate empty and `4a Armed` has already left the card behind.
   *
   * So it is chosen, and the constraint chooses most of it: `05-deletions.md` says the armed
   * takeover is "the only place in the app a solid-red fill appears", which rules out promoting this
   * to `destructive`. What is left is the disabled button with the two things disabled-ness takes
   * away put back — the label moves from the muted `#8A5A5A` to the live `--destructive-text`, and
   * the hairline from the .25 button border to the .38 card border. Same fill, so the button does
   * not jump; a brighter word and a brighter edge, so it is visibly ready. DEVIATIONS §75.
   */
  destructiveArmable: {
    bg: "var(--btn-destructive-disabled-bg)",
    border: "var(--destructive-border)",
    color: "var(--destructive-text)",
    weight: 600,
  },
};

// ------------------------------------------------------------------------------- the sizes ----

/**
 * The six rungs that cover 100 of the 150 drawn buttons. Everything else overrides.
 *
 * Named for where they sit rather than by t-shirt size: `bar` is the footer action bar, `compact`
 * the 360px panel, `icon` the 30×30 `⋯` and the `‹ ›` steppers.
 */
const SIZE = {
  bar: { radius: "var(--r-10)", padding: "11px 22px", fontSize: "13px" },
  standard: { radius: "var(--r-9)", padding: "9px 16px", fontSize: "12.5px" },
  small: { radius: "var(--r-8)", padding: "7px 14px", fontSize: "12px" },
  compact: { radius: "var(--r-7)", padding: "5px 11px", fontSize: "11.5px" },
  pill: { radius: "var(--r-pill)", padding: "7px 15px", fontSize: "12.5px" },
  icon: { radius: "var(--r-8)", padding: "1px 6px", fontSize: "15px" },
  /**
   * The `3a Conflict` choice buttons, and the one rung that sets NO font-size.
   *
   * `null` is not "unmeasured" — it is the measurement. The frames record 13.3333px on all three
   * elements, which is the UA's own button default and not a design decision: the label span sets
   * 13.5px and the consequence sentence 12px, so nothing inherits the element's size. base.css
   * resets a button's font-FAMILY but not its size, so saying nothing here reproduces it exactly,
   * the same reasoning that keeps `display` and `line-height` out of `.btn`.
   */
  choice: { radius: "var(--r-13)", padding: "15px 17px", fontSize: null },
};

/** The glyph's colour is per-instance, not per-kind: `→` is warm, `←` is cool, on the same tint. */
const GLYPH_TONE = {
  up: "var(--up-label)",
  down: "var(--down-label)",
  onPrimary: "var(--btn-primary-text)",
};

/**
 * A button.
 *
 * `kind` is the colour role and carries the font weight; `size` is the geometry. Anything in the
 * long tail passes `padding`, `radius` or `fontSize` directly rather than earning a rung.
 *
 * Disabled is a KIND, not a state flag, because the design gives disabled its own fill and text
 * rather than dimming the enabled one — `primaryDisabled` is `#2A2E36`/`#6D7783`, not the primary at
 * reduced opacity. Passing `disabled` sets the attribute and the cursor; it does not recolour.
 */
export function button({
  kind = "secondary",
  size = "standard",
  label,
  sublabel = null,
  glyph = null,
  glyphTone = null,
  onClick = null,
  disabled = false,
  padding = null,
  radius = null,
  fontSize = null,
  class: cls = null,
  ...rest
} = {}) {
  const role = KIND[kind];
  if (!role)
    throw new Error(`controls: unknown button kind "${kind}". Known: ${Object.keys(KIND).join(", ")}`);
  const rung = SIZE[size];
  if (!rung)
    throw new Error(`controls: unknown button size "${size}". Known: ${Object.keys(SIZE).join(", ")}`);
  if (glyphTone && !GLYPH_TONE[glyphTone])
    throw new Error(
      `controls: unknown glyph tone "${glyphTone}". Known: ${Object.keys(GLYPH_TONE).join(", ")}`,
    );

  // TWO SHAPES, and the glyph is what picks between them.
  //
  // Every button in the app is a label and maybe a quieter second line. A CHOICE button is a
  // glyphed row over a consequence sentence, and the frames build it out of two blocks whose spans
  // each carry their own colour — which is why the element itself ends up unpainted. Reproducing
  // that structure is not cosmetic: `data-fid` keys are element paths, so a `span` where the design
  // draws a `div` is a key that does not exist and a gate that cannot check the node at all.
  const body = glyph
    ? [
        el(
          "div",
          { class: "btn-choice-row" },
          el("span", { class: "btn-glyph" }, glyph),
          el("span", { class: "btn-choice-name" }, label),
        ),
        sublabel ? el("div", { class: "btn-sub btn-choice-sub" }, sublabel) : null,
      ]
    : [
        label,
        // `Keep both` carries its consequence inside the button, in a quieter tier. It is the one
        // place the design puts two lines in a control, and it is deliberate: the sentence is the
        // reason the loud button is the safe one.
        sublabel ? el("span", { class: "btn-sub" }, sublabel) : null,
      ];

  const node = el(
    "button",
    {
      class: ["btn", `btn-${kind}`, glyph ? "btn-choice" : null, cls].filter(Boolean).join(" "),
      type: "button",
      disabled: disabled || role.disabled || null,
      onClick: disabled || role.disabled ? null : onClick,
      ...rest,
    },
    ...body,
  );

  node.dataset.kind = kind;
  node.style.borderRadius = radius ?? rung.radius;
  node.style.padding = padding ?? rung.padding;
  // A rung may decline to set a size (`choice`), in which case the element keeps the UA default —
  // see SIZE.choice. Assigning null here would serialise as `""` and remove the declaration, which
  // happens to work; the explicit guard says it is intended rather than tolerated.
  const resolvedSize = fontSize ?? rung.fontSize;
  if (resolvedSize != null) node.style.fontSize = resolvedSize;
  applyKind(node, kind, glyphTone);
  return node;
}

/**
 * Repaint an existing button as a different KIND, in place.
 *
 * For the one control that changes role without changing anything else about itself: the deletion
 * gate's `Delete`, which goes from `destructiveDisabled` to `destructiveArmable` the instant the
 * typed word matches. Rebuilding it there would mean re-rendering the card the field is being typed
 * into, and toggling a class cannot do it — every colour a kind carries is written as an INLINE
 * custom property, which no stylesheet rule can reach past.
 *
 * The KIND TABLE STAYS THE ONE SOURCE. This is the same `applyKind` the constructor calls, so a
 * kind gains a property in exactly one place and both paths get it.
 */
export function setButtonKind(node, kind, glyphTone = null) {
  applyKind(node, kind, glyphTone);
  node.classList.remove(`btn-${node.dataset.kind}`);
  node.classList.add(`btn-${kind}`);
  node.dataset.kind = kind;
  node.disabled = Boolean(KIND[kind].disabled);
}

function applyKind(node, kind, glyphTone) {
  const role = KIND[kind];
  if (!role)
    throw new Error(`controls: unknown button kind "${kind}". Known: ${Object.keys(KIND).join(", ")}`);
  node.style.fontWeight = String(role.weight);

  // Colour goes through CUSTOM PROPERTIES rather than straight onto `background`/`color`, so
  // controls.css can express `:hover` and `:active` at all. An inline `background` beats any
  // stylesheet rule, so the obvious version leaves the hover state unreachable — and the shortcut
  // around that, a `filter: brightness()`, is not what the design says: 14-behaviour-and-state.md
  // §"Interactive states" steps the SURFACE up one level and the BORDER up one, leaving the text
  // alone. A brightness filter would lift the label too, which on a primary means washing out the
  // one deliberately maximum-contrast thing on the screen.
  node.style.setProperty("--btn-fg", role.color);
  node.style.setProperty("--btn-bg", role.bg ?? "transparent");
  // A KIND WITH NO BORDER DRAWS NO BORDER — style `none`, not a 1px transparent one, and the colour
  // a browser reports for a border nobody draws is `currentColor`. Both halves matter to the gate:
  // `1px solid transparent` is two extra pixels in the border box (`Delete permanently` measured
  // 172.27×42 against the drawn 170.27×40) and `transparent` is not the `rgb(255, 255, 255)` the
  // frame records for the same edge. Found on `4a Armed`, true of every borderless kind.
  node.style.setProperty("--btn-border", role.border ?? "currentColor");
  node.style.setProperty("--btn-bg-hover", role.bgHover ?? role.bg ?? "transparent");
  node.style.setProperty("--btn-border-hover", role.borderHover ?? role.border ?? "currentColor");
  // Press steps up ONE MORE level than hover. `--panel-raised` is the top of the surface ladder, so
  // a kind already sitting there presses to `--border` — the next thing up, and still not a
  // brightness change to the label.
  node.style.setProperty("--btn-bg-active", role.bgActive ?? role.bgHover ?? role.bg ?? "transparent");
  // The three properties a KIND may or may not own. Set when it declares one and REMOVED when it
  // does not — because `setButtonKind` repaints in place, and a property left behind from the
  // previous kind is a colour nothing in the new one asked for. Nothing repaints across a pair that
  // differs in these today; the clear is here so that the day one does, it does not inherit.
  setOrClear(node, "--btn-label-fg", role.labelColor);
  setOrClear(node, "--btn-sub-fg", role.subColor);
  // A kind that declares no border also declares no border STYLE, so the pair above resolves to
  // `1px none currentColor` and the border box stays the element's own size.
  setOrClear(node, "--btn-border-style", role.borderStyle ?? (role.border ? null : "none"));
  // NOT cleared, because it is not the kind's. The glyph tone is per-instance — `→` is warm and `←`
  // cool on the same tint — so a repaint that was given none leaves the instance's own alone.
  if (glyphTone) node.style.setProperty("--btn-glyph-fg", GLYPH_TONE[glyphTone]);
  return node;
}

function setOrClear(node, property, value) {
  if (value) node.style.setProperty(property, value);
  else node.style.removeProperty(property);
}

// ------------------------------------------------------------------------------ text inputs ----

/**
 * A text input. `mono` for anything holding a path or a rule — the design puts every filename,
 * pattern and daemon string in IBM Plex Mono, and a skip rule typed into a sans field reads as prose.
 */
export function textInput({ value = "", placeholder = "", mono = false, onInput = null, ...rest } = {}) {
  return el("input", {
    class: "input" + (mono ? " is-mono" : ""),
    type: "text",
    value,
    placeholder,
    onInput,
    ...rest,
  });
}

/**
 * The typed-`DELETE` gate. `14-behaviour-and-state.md`: case-sensitive, clears on blur, and the
 * button stays disabled until the word matches.
 *
 * CLEARING ON BLUR IS THE POINT, not a nicety. The gate exists so that deleting something
 * irreversible takes a deliberate act in the same moment as the click — a field left armed while
 * you go and do something else has stopped being a gate and become a formality.
 *
 * Case-sensitive for the same reason: `delete` is a word people type by habit, `DELETE` is not.
 * The copy deck spells the word in caps everywhere it appears, and it is the one place voice rule 5
 * (no shouting) is deliberately broken.
 *
 * BUT BLUR IS NOT THE SAME AS LEAVING, and the first version of this made the gate impossible to
 * complete. Reaching the button the gate unlocks blurs the field on the way — so the field cleared,
 * `onChange(false)` disabled the button, and the click already in flight landed on a disabled
 * control. Confirmed for the POINTER (mousedown blurs before click dispatches) and for the KEYBOARD
 * (Tab blurs before Enter), so the only irreversible action in the app could not be performed at
 * all.
 *
 * So the field defers to its group: a blur whose `relatedTarget` is inside `[data-delete-gate]` is
 * the second half of the same act, not an abandonment. `deletionGate` in rows.js stamps that
 * attribute and watches the group's own `focusout`, which is where "you went and did something
 * else" is actually observable. The rule the design asks for is unchanged — it is enforced at the
 * boundary that means it. DEVIATIONS §55a.
 */
export function deleteGate({ onChange = null, word = "DELETE" } = {}) {
  const field = textInput({
    placeholder: word,
    mono: true,
    "aria-label": `Type ${word} to confirm`,
    onInput: (e) => onChange?.(e.target.value === word),
    onBlur: (e) => {
      if (e.target.value === "") return;
      // `closest` and not `contains`: relatedTarget is the node RECEIVING focus, which is the
      // confirm button — a sibling of this field, not a descendant of it.
      if (e.relatedTarget?.closest?.("[data-delete-gate]")) return;
      e.target.value = "";
      onChange?.(false);
    },
  });
  field.classList.add("delete-gate");
  return field;
}

/**
 * Mark a node as the gate's group: the region a typed word may move within, and outside which it is
 * abandoned. Used by S3's deletion card and S4's footer bar — one copy, so the two cannot drift.
 *
 * Attribute and listener are halves of one rule. `deleteGate`'s blur consults `[data-delete-gate]`
 * so that reaching the button is not leaving; this `focusout` is what sees focus leave the group
 * entirely, which the field's blur cannot — focus is already on the button by then. Attribute
 * without listener: an abandoned word stays armed for the next click. Listener without attribute:
 * the gate cannot be completed at all.
 *
 * The cleared value goes back through a dispatched `input` event rather than a direct repaint, so
 * there is one path from what the field holds to what the button looks like.
 */
export function gateGroup(node) {
  if (!node) return node;
  node.dataset.deleteGate = "";
  node.addEventListener("focusout", (e) => {
    // Still inside the group — moving between the field and its button is not leaving.
    if (node.contains(e.relatedTarget)) return;
    const field = node.querySelector(".delete-gate");
    if (!field || field.value === "") return;
    field.value = "";
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
  return node;
}

// --------------------------------------------------------------------------------- toggles ----

/** The 44×26 toggle. Two drawn states and nothing between them. */
export function toggle({ on = false, onChange = null, label = null } = {}) {
  const knob = el("span", { class: "toggle-knob" });
  const node = el(
    "button",
    {
      class: "toggle" + (on ? " is-on" : ""),
      type: "button",
      role: "switch",
      "aria-checked": String(on),
      "aria-label": label,
      onClick: () => onChange?.(!on),
    },
    knob,
  );
  return node;
}

/**
 * The 17px checkbox. `9a Consent`'s "I understand deletions travel both ways." is the drawn one.
 *
 * NO STATE CLASS ON THE BOX, and this is the one control in the module where that is right. The
 * visual state comes from `.checkbox-input:checked + .checkbox-box` in controls.css — the real
 * `<input>` is the source of truth, and the browser updates it on every interaction for free.
 *
 * An `is-checked` class alongside it was a SECOND source of truth that only ever recorded the value
 * of the `checked` argument at build time: tick the box and the class stayed as it was. Nothing
 * consumed it, so nothing was visibly wrong — it was a trap for whoever styled it next.
 *
 * `toggle`'s `is-on` and `radioCard`'s `is-selected` are NOT the same thing and must stay. Neither
 * has an `<input>` under it — a toggle is a `<button role="switch">`, a radio card a
 * `<div role="radio">` — so there the class IS the state, and the caller re-renders to change it.
 * The difference is whether the DOM already knows.
 */
export function checkbox({ checked = false, label, onChange = null } = {}) {
  const box = el("span", { class: "checkbox-box" });
  return el(
    "label",
    { class: "checkbox" },
    el("input", {
      type: "checkbox",
      class: "checkbox-input",
      checked: checked || null,
      onChange: (e) => onChange?.(e.target.checked),
    }),
    box,
    el("span", { class: "checkbox-label" }, label),
  );
}

// ------------------------------------------------------------------- segmented / tabs / chips ----

/**
 * Pill tabs — `Files · Sync passes · Details`, `Folders · What to skip · Deletions · Advanced`.
 * The selected one is a primary fill that keeps its border (`primaryOutlined`), which is the only
 * place that kind appears.
 */
export function pillTabs({ items, active, onSelect }) {
  return el(
    "div",
    { class: "pill-tabs", role: "tablist" },
    items.map((item) =>
      button({
        kind: item.id === active ? "primaryOutlined" : "secondary",
        size: "pill",
        label: item.label,
        onClick: () => onSelect(item.id),
        role: "tab",
        "aria-selected": String(item.id === active),
      }),
    ),
  );
}

/**
 * A segmented control — `Weekly` / `Monthly` on the full-sweep schedule (#193).
 *
 * A DIFFERENT SHAPE FROM [`pillTabs`], not a variant of it: the pills are a bare flex row of
 * bordered buttons, this is a bordered *track* the selected segment sits inside. `8a Settings`
 * draws the track `#0A0B0D` inside a `#23262D` border at `radius:10 padding:3 gap:3`, the selected
 * segment as `primary` at `radius:7`, and the unselected one with no border and no background.
 *
 * `flex-shrink: 0` matters: the track sits beside a text block that takes the rest of the head row,
 * and without it the segment labels wrap at the width the frame draws.
 */
export function segmentedControl({ items, active, onSelect }) {
  return el(
    "div",
    { class: "segmented", role: "tablist" },
    items.map((item) =>
      button({
        kind: item.id === active ? "primary" : "segment",
        label: item.label,
        onClick: () => onSelect(item.id),
        padding: "6px 14px",
        radius: "var(--r-7)",
        fontSize: "12px",
        role: "tab",
        "aria-selected": String(item.id === active),
      }),
    ),
  );
}

/** Day chips — `Mon Tue Wed …`, drawn with no horizontal padding and a fixed width. */
export function dayChips({ days, selected = [], onToggle, className = null }) {
  return el(
    "div",
    { class: className ? `day-chips ${className}` : "day-chips" },
    days.map((day) =>
      button({
        kind: selected.includes(day) ? "primaryOutlined" : "secondary",
        size: "small",
        padding: "6px 0",
        label: day,
        onClick: () => onToggle(day),
        "aria-pressed": String(selected.includes(day)),
      }),
    ),
  );
}

/**
 * The `− ＋` stepper. Both glyphs are drawn as filled secondaries at icon size.
 *
 * `＋` IS THE FULLWIDTH PLUS, U+FF0B, and it is what both frames that draw a stepper carry — the
 * ASCII `+` sits high and thin beside a `−` that is already the typographic minus. The icon rung's
 * 15px is the `⋯` menu's size and would draw these two a size loud, so the glyphs set 13px, which
 * is what `8a Settings` and `8a Schedule monthly` both measure.
 */
export function stepper({ value, onStep, format = String, min = null, max = null } = {}) {
  const at = (delta) => (delta < 0 ? min != null && value <= min : max != null && value >= max);
  const step = (label, delta) =>
    button({
      kind: "secondaryFilled",
      size: "icon",
      fontSize: "13px",
      label,
      disabled: at(delta),
      onClick: () => onStep(delta),
    });
  return el(
    "div",
    { class: "stepper" },
    step("−", -1),
    el("span", { class: "stepper-value mono" }, format(value)),
    step("＋", 1),
  );
}

/**
 * Radio cards — Settings › Deletions' three policies.
 *
 * REBUILT FROM THE FRAME BY S6, and the shape changed. F5 wrote it from
 * `14-behaviour-and-state.md`'s prose as `card > [ring, body > [title, text]]`; `8a Deletions tab`
 * draws `card > [head > [ring, title, badge], text]` — the ring is INSIDE a flex head row with the
 * title and the `recommended` badge, and the body is that row's sibling, indented past the ring by
 * a 26px padding rather than by being nested under it. Nothing had rendered one, so nothing
 * disagreed: the fourth primitive in this module to be corrected by the first screen to draw it.
 *
 * The prose and the frame also disagree about the unselected ring — 1.5px in `08-settings.md`, 1px
 * on the frame. The frame wins, as it does for the 13-to-6 footer split (§40). DEVIATIONS §78.
 */
export function radioCard({
  selected = false,
  title,
  note = null,
  body = null,
  onSelect = null,
  tone = null,
}) {
  return el(
    "div",
    {
      class: ["radio-card", selected ? "is-selected" : null, tone ? `tone-${tone}` : null]
        .filter(Boolean)
        .join(" "),
      role: "radio",
      tabindex: "0",
      "aria-checked": String(selected),
      onClick: () => onSelect?.(),
      onKeydown: (e) => {
        if (e.key === " " || e.key === "Enter") {
          e.preventDefault();
          onSelect?.();
        }
      },
    },
    el(
      "div",
      { class: "radio-head" },
      el("span", { class: "radio-ring" + (selected ? " is-selected" : "") }),
      el("span", { class: "radio-title" }, title),
      note ? el("span", { class: "radio-note" }, note) : null,
    ),
    body ? el("div", { class: "radio-text" }, body) : null,
  );
}
