# craft-desktop (local integration snapshot)

Portable menu snapshots + opt-in Linux DBusMenu/window binding and portal appearance.
No app or egui dependency. The caller must revalidate queued actions on its UI thread
and retain its local menu. Enable `linux` in the desktop host, `portal` for appearance;
the default model-only feature set builds without Linux dependencies (including wasm).

Canonical source: workspace `shared/craft-desktop`; identical copies in each app's
`vendor/craft-desktop` keep independent checkouts and Flatpak builds self-contained.
The root winit patch must point to the adjacent `vendor/winit` snapshot. Patches in
this dependency's manifest do not propagate to downstream workspaces.

The transport derives from the pre-existing WordCraft DBusMenu implementation,
under the same MIT OR Apache-2.0 licence. Native Wayland uses a minimal additive safe
winit accessor; see `../winit/CRAFT_PATCH.md` for provenance and maintenance.
