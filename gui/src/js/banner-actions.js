// What a banner's buttons do — as a sequence, with every effect handed in (#102 phase 5e).
//
// WHY THIS IS NOT IN `app.js`. The order of the steps is the whole behaviour, and `app.js` boots on
// import, so nothing in Node can run it. At one folder the order never mattered. With two or more, a
// banner is about ONE of them, and a click must act on that one however long after the banner went up
// it lands and whatever the window is showing by then:
//
//   · `review` / `compare` open a screen. The window has to be ABOUT the banner's folder before it goes
//     there, or it draws the folder that happened to be selected — a Deletions queue with nothing on the
//     screen saying whose it is (brief A10, J20). So the folder is selected first and the screen is
//     navigated to only once that has happened; a folder that cannot be selected (it is not run any
//     more) opens the window and goes nowhere, because a navigation to the wrong folder's queue is a
//     worse answer than the window alone.
//   · `retry` and `keep` do not move the window at all. They are addressed to the banner's folder by
//     name, as every write is (`api.js`): `keep` keeps that folder's permanent deletions and no other's,
//     `retry` is a `syncnow` for that folder and not for every unpaused one.
//   · `open` is `Open Drive Sync` on the outage banner: the window, on the folder the outage is about.
//
// `pair` is absent at one folder, where every action is exactly what it was.

/**
 * Run one banner action.
 *
 * `fx` is `{ keep(folder|null), tryAgain(), syncPair(folder), open(), select(folder) → Promise<boolean>,
 * navigate(route), warn(message) }`. `select` answers whether the folder IS selected now.
 */
export async function runBannerAction({ kind, action, pair = null } = {}, fx) {
  const folder = typeof pair === "string" && pair !== "" ? pair : null;
  switch (action) {
    case "keep":
      return fx.keep(folder);
    case "later":
      // Dismiss. The thing is still in the window, which is the whole design of this action.
      return undefined;
    case "retry":
      return folder ? fx.syncPair(folder) : fx.tryAgain();
    case "compare":
    case "review":
      fx.open();
      if (folder && !(await fx.select(folder))) return undefined;
      return fx.navigate(kind === "deletion" ? "deletions" : "conflicts");
    case "open":
      fx.open();
      if (folder) await fx.select(folder);
      return undefined;
    default:
      return fx.warn(`notification-action: no handler for "${action}"`);
  }
}
