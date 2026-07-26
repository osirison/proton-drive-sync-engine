// Screen registry (F2). Slots + nav are pre-declared here and each screen's `render` lives in its
// OWN module under ./screens/ — so parallel screen tasks (S1–S11) only ever edit their own file and
// never this registry, the router, or each other. To add a screen: create ./screens/<id>.js, import
// its render here, and add one row. (Tray S7 and file-manager emblems S10 are NOT webview screens
// and live outside this registry.)

import { renderOverview } from "./screens/overview.js";
import { renderActivity } from "./screens/activity.js";
import { renderConflicts } from "./screens/conflicts.js";
import { renderDeletions } from "./screens/deletions.js";
import { renderPlan } from "./screens/plan.js";
import { renderHistory } from "./screens/history.js";
import { renderSettings } from "./screens/settings.js";

export const SCREENS = [
  { id: "overview", label: "Overview", icon: "◇", banner: true, issue: "S1 #82", render: renderOverview },
  { id: "activity", label: "Activity", icon: "≡", counter: true, issue: "S2 #83", render: renderActivity },
  { id: "conflicts", label: "Conflicts", icon: "⚠", badge: true, issue: "S3 #84", render: renderConflicts },
  { id: "deletions", label: "Deletions", icon: "⊘", badge: true, issue: "S9 #90", render: renderDeletions },
  { id: "plan", label: "Plan preview", icon: "▤", banner: true, issue: "S4 #85", render: renderPlan },
  { id: "history", label: "History", icon: "◷", issue: "S5 #86", render: renderHistory },
  { id: "settings", label: "Settings", icon: "⚙", banner: true, issue: "S6 #87", render: renderSettings },
];
