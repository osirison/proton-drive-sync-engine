---
trigger: qdbus org.kde.KWin, loadScript, kwin script, app_id, resourceClass, WM_CLASS, StartupWMClass, kiconfinder6, taskbar icon, skip_taskbar, always_on_top, keep_above, GDK_BACKEND, wayland hint discarded
depends_on: gui/src-tauri/src/lib.rs, gui/src-tauri/src/panel.rs, gui/src-tauri/tauri.conf.json, packaging/freedesktop/app.protondrivesync.engine.desktop
recorded: 2026-08-23 (two-backend control added 2026-08-29)
---

# Reading a GUI window's Wayland `app_id` on KDE — the KWin script's `print` goes nowhere

Applies to any check on how the desktop matches `proton-sync-gui`'s windows to its `.desktop` file
(the taskbar icon, Alt-Tab, `StartupWMClass`). A native Wayland toplevel has no X11 properties, so
`xprop`/`xdotool`/`wmctrl` cannot see it — they are installed and do still work against an XWayland
client or anything forced to `GDK_BACKEND=x11`, which this app is not.

## The precondition nothing states

KWin's scripting API is the only way in, and it is loaded over D-Bus:

```bash
qdbus org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript /abs/path/to/dump.js myname   # → N
qdbus org.kde.KWin /Scripting/ScriptN org.kde.kwin.Script.run
```

Two traps:

- **`loadScript` returns the script number, and the object path is `/Scripting/Script<N>` — not the
  name you passed.** Capture the return value; do not guess `Script0`.
- **`print()` inside the script goes to the `kwin_scripting` logging category, which is off.**
  Nothing reaches `journalctl`. Throwing instead does reach it, at warning level:

```js
var out = [];
var ws = workspace.windowList();          // Plasma 6. Plasma 5: workspace.clientList()
for (var i = 0; i < ws.length; i++) {
  var w = ws[i];
  out.push(w.resourceClass + " ~ " + w.desktopFileName + " ~ " + w.caption
    + " skipTaskbar=" + w.skipTaskbar);
}
throw new Error("DUMP >>> " + out.join(" || "));
```

```bash
journalctl --user --since "-20 s" --no-pager | grep -o "DUMP.*"
```

A script object is unloaded once it has run, so a second reading needs a second `loadScript`.

`w.icon` is exposed but `w.icon.name` reads `undefined`, so the resolved icon cannot be read this
way. Resolve it the way Qt does instead:

```bash
kiconfinder6 app.protondrivesync.engine   # exit 1 + no output = KWin will use its fallback
```

## The same reading proves a toolkit call inert, if you run a control

A GTK call that Wayland has no protocol for does not error — it does nothing, and the code reads as
if it worked. One reading cannot tell that apart from a call that worked; **two windows differing
only in `GDK_BACKEND` can**, because the X11 one is the control that shows the call is issued
correctly.

```python
# probe.py — no Tauri, no app, no account: plain GTK3 issuing the calls under test.
import sys, gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gtk, GLib
w = Gtk.Window(title="SKIPPROBE-" + sys.argv[1])
w.set_skip_taskbar_hint(True); w.set_skip_pager_hint(True); w.set_keep_above(True)
w.show_all()
GLib.timeout_add_seconds(25, Gtk.main_quit)
Gtk.main()
```

```bash
GDK_BACKEND=wayland python3 probe.py wayland &
GDK_BACKEND=x11     python3 probe.py x11 &
sleep 4   # both must be mapped before KWin is asked
```

Then the `loadScript`/`run`/`journalctl` sequence above, with the dump filtered to `SKIPPROBE` and
reporting `skipTaskbar`, `skipPager`, `skipSwitcher`, `keepAbove`. Result on Plasma 6 (#351):

| `GDK_BACKEND` | `skipTaskbar` | `skipPager` | `keepAbove` | `skipSwitcher` |
| --- | --- | --- | --- | --- |
| `x11` | `true` | `true` | `true` | `false` |
| `wayland` | `false` | `false` | `false` | `false` |

Two things only the control gives you. **The Wayland row alone is not evidence** — it is equally
consistent with the probe never issuing the call. And a property that is `false` on *both* rows
(`skipSwitcher` here) is not a Wayland gap at all: it is a call that sets something else, which is
how a comment ends up claiming an effect the code never had on any platform.

Map the toolkit call to the Tauri builder method through tao's dispatch — `grep WindowRequest::`
in `tao-*/src/platform_impl/linux/event_loop.rs` — so the probe tests what the app actually issues
rather than something adjacent.

## Why it matters here

`resourceClass` **is** the Wayland `app_id`, and KWin turns it into an icon by looking up
`<app_id>.desktop`; a miss falls back to `QIcon::fromTheme("wayland")` — breeze's amber "W" disc,
which looks like a broken app rather than a naming mismatch. `gui/src-tauri/src/lib.rs`'s
`adopt_launcher_identity` is what makes the two agree, and this is how to confirm it still does.

Do **not** confirm it with a screenshot of the app: the window carries a real Drive.
