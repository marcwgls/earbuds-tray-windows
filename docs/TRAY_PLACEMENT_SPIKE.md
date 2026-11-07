# Spike: tray icons visible right after a fresh install

Question: can the three icons appear directly in the taskbar (not under ^) on a new install, without the user enabling them in Settings?

## Findings so far (2026-10-31, test machine, build 26300)

- Windows 11 stores one entry per tray icon under `HKCU\Control Panel\NotifyIconSettings\<hash>` with `ExecutablePath`, `UID`, `IconGuid`, `InitialTooltip` and `IsPromoted` (1 = shown in the taskbar). The Settings toggle writes `IsPromoted`.
- Our three entries: UID 2, 4, 6, all `IsPromoted = 1` (enabled by hand), no `IconGuid`.
- The UID comes from tray-icon's global `COUNTER`, so it depends on creation order. UID 2 still has the Phase 3 tooltip, so the Case icon most likely inherited the old single icon's entry. Identity is accidental.
- tray-icon 0.26.1 has `TrayIconBuilder::with_guid(u128)` (sets `NIF_GUID`); then Windows identifies the icon by GUID instead of exe path + UID.

## Open questions

1. Does Explorer apply a change of `IsPromoted` live, or only after an Explorer restart or app restart?
2. When a new icon appears, does Windows create the entry with `IsPromoted` missing or 0? Can the app then set it to 1 itself (only if missing, never override the user's 0)?
3. With `with_guid`: does a changed exe path (debug vs release vs installed) break `NIM_ADD` (older Windows bound a GUID to one exe path)?

## Experiments

- A (question 1, done by hand, reversible): while the app runs, set Left's `IsPromoted` to 0, watch the tray, set it back to 1.
  Result: works live. 0 moved L into the overflow at once and it stayed there, 1 brought it back at once. Question 1 answered: yes, no restart needed.
- B (question 2): close the app, delete our three entries (simulates fresh install), start the app, read the new entries.
  Result so far: after deleting our entries and restarting the app, the icons land under ^ (fresh install behavior reproduced). While the app runs, Explorer has written no new entries yet (29 entries, before 32), so it keeps them in memory first. Not on Settings page, not on app quit, not on dragging L into the taskbar (also no trace anywhere in HKCU). Entries appeared right after an Explorer restart (Task Manager), for all three icons, with `IsPromoted` missing, even for the dragged L. So Explorer persists on its own exit (restart, logoff). Fresh install consequence: in the first session nothing can be promoted; from the next login on the entries exist without `IsPromoted` and the app can set it.
- C (question 3): only after A and B, in code.

Decision after the experiments: if A and B work, plan a small phase with fixed GUIDs and a "promote once on first start" step. If not, fall back to a first-start hint (balloon/tooltip) telling the user where to enable the icons.

Follow-up: after the Explorer restart the dragged L was back under ^ (the drag was never persisted). Then set `IsPromoted = 1` on all three entries where it was missing, which is exactly what the app would do.

## Current conclusion

- The app cannot promote in the very first session of a fresh install (no entry exists yet).
- From the next login (or Explorer restart) on, the app can find its entries (missing `IsPromoted`) and set 1, effective live.
- Dragging an icon is not persisted by Explorer on this build, only the Settings toggle is. So "promote once, only if missing" keeps the user's explicit Settings choice (0) intact.
- Plan candidate: fixed GUIDs per icon, a watcher that checks the entries at startup and periodically, a first-start hint for the first session.

- D (installer idea): delete entries, start app, hard restart Explorer (`Stop-Process -Name explorer -Force`, then start it). Result: all three entries exist afterwards, `IsPromoted` missing, so even a forced restart is enough (most likely the new Explorer writes them when the app re-adds its icons via `TaskbarCreated`). Then set to 1 again.
  Installer flow: install, start app as the original user, restart Explorer as that user (not elevated), app promotes its entries. Icons visible in the first session.

## Correction (2026-11-01, Phase 9)

A real fresh install (new exe path, twice) gets its `NotifyIconSettings` entries written by Explorer immediately, without any restart, and the app promotes them live. The "entries appear only after an Explorer restart" result above came from deleting our entries while Explorer still held them in memory. No Explorer restart is needed in the installer.
