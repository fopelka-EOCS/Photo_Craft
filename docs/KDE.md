# KDE Plasma / Linux

Updated 2026-10-21. Linux exports the existing local command tree through
`vendor/craft-desktop`, without hiding the local menu. The adapter rechecks the active
document, dynamic recent/document targets, modal state and enabled state before calling
the normal interactive dispatcher. Keyboard shortcuts and checked states share the local model.

Native Wayland uses KDE's appmenu protocol on the existing window surface; the vendored
winit extension returns a safe cloned proxy without transferring connection or surface
ownership. X11/XWayland uses window properties and `com.canonical.AppMenu.Registrar`.
Keep both vendor directories and the root Cargo winit patch in source distributions;
see [the patch record](../vendor/winit/CRAFT_PATCH.md). No Qt conversion or GTK module is needed.

## Build and launch

```sh
cargo build -p photocraft
photo_kde_config=$(mktemp -d)
# In a Plasma Wayland session, prevent the tablet policy from selecting XWayland:
env -u DISPLAY PHOTOCRAFT_CONFIG_DIR="$photo_kde_config" cargo run -p photocraft -- --new-instance
# In a session with an X display, test X11/XWayland separately:
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET PHOTOCRAFT_CONFIG_DIR="$photo_kde_config" cargo run -p photocraft -- --new-instance
```

Run these separately, closing the first test window before the second. Use disposable
documents and keep the temporary configuration for inspection. Do not reuse real settings
or documents. Existing tablet/display-server policy is unchanged; native Wayland file
drag-and-drop remains a separate winit limitation, and XWayland may blur fractional scaling.

Add Plasma's **Global Menu** widget manually if needed. With no widget, registrar, session
bus or compositor protocol, the local menu remains available. `PHOTOCRAFT_NO_GLOBAL_MENU=1`
or `KDE_NO_GLOBAL_MENU=1` disables export. No desktop configuration is modified automatically.

## Reproducible validation

```sh
cargo fmt --all -- --check
cargo check -p photocraft
cargo clippy -p photocraft -p photocraft-ui-egui --all-targets -- -D warnings
cargo test -p photocraft-ui-egui native_menu
cargo test -p photocraft-ui-egui desktop_menu_
cargo test -p photocraft-ui-egui scale_preferences_and_monitor_changes_apply_live
cargo test -p photocraft linux_new_windows_use_desktop_decorations_without_overwriting_saved_choices
cargo test --manifest-path vendor/craft-desktop/Cargo.toml --features linux,portal
dbus-run-session -- cargo test --manifest-path vendor/craft-desktop/Cargo.toml --features linux,portal appmenu_dbus -- --ignored
```

Check native Wayland and X11 separately:

1. Focus the editor and another application; the panel must show the focused application's
   textual menus. Open New/Open/Save As/Preferences from the panel and cancel each dialog.
2. With disposable documents, toggle Rulers, perform an edit, Undo/Redo and compare checked
   and enabled states with the local menu. Check selection-dependent commands too.
3. Switch between two documents, close one, and reject delayed old-document/menu-slot
   clicks. Close a dirty document and cancel the normal save prompt. Open a modal dialog;
   document-changing panel commands must be unavailable.
4. Disable the exporter and test without a widget; local menus remain functional. Test
   menu-service restart/re-registration manually, not by restarting the user's shell in a script.
5. Verify a new configuration uses KWin's title bar, and explicit System Title Bar choices
   survive an application restart. In Automatic appearance change the desktop light/dark
   setting manually, then verify a fixed Breeze choice stays fixed.
6. At 100/125/150/175/200%, resize and inspect text, long menus, dialogs and cursor alignment.
   Move between different-DPI monitors: Auto follows each monitor; fixed UI Scale is absolute.
7. Open/save/cancel through the KDE portal picker. Repeat the checklist in packaged Flatpak;
   registrar permission alone does not establish sandbox/runtime acceptance.

See [UI design](ui-design.md#high-dpi-and-4k-displays) for appearance and scaling semantics.
Protocol/model tests and compilation do not certify visible Plasma rendering, physical monitor
transitions or portal interaction. The [Recorded verification](#recorded-verification-2026-10-21)
section lists the checks actually run; the checklist is not a claim of comprehensive GUI acceptance.

## Recorded verification (2026-10-21)

The Linux production build, `cargo check` and app/UI `cargo clippy --all-targets -- -D warnings`
passed. Targeted unit tests also passed: four `desktop_menu_` adapter regressions, eighteen
existing `native_menu` regressions, one live-scale preference test and two host titlebar
policy tests (25 distinct tests, zero failures). The app and UI test packages were built
together to retain production feature unification; the already-built executables were
reused for the remaining filters. The full 3191-test UI and 191-test host suites were not run.

In an isolated configuration on the host's real Plasma Wayland session:

- Plasma was observed consuming the exported menu through `GetLayout` and `AboutToShow`;
  D-Bus integration checks exercised its `Event` path. Native Wayland association was
  confirmed on the existing surface, without an X11 window ID.
- The ten text sections were File, Edit, Image, Layer, Type, Select, Filter, View, Window
  and Help. Activating the exported Rulers action changed its checked state from 0 to 1; opening About
  disabled Rulers in the exported menu.
- A fresh profile reported System Title Bar enabled. Automatic dark and explicit light
  appearance screenshots (1872 × 1053, About open) were visually inspected.
- Sending the exported Close action for a dirty disposable document opened the normal discard
  confirmation for `file.close`; Cancel retained the document. The exported service
  disappeared after application quit.

Runtime logs/screenshots for this run were local, temporary test artifacts, not
committed fixtures. The observations above summarize the native-session checks.
These observations do not establish physical multi-monitor transitions, every scale,
X11 behavior or packaged Flatpak acceptance. Targeted unit-test results are separate
from compilation and native-session observations.
