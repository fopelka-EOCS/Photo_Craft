//! Geometry regressions for the Preferences and Keyboard Shortcuts modals.

use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn harness(size: egui::Vec2, preferences: bool) -> Harness<'static, PhotocraftApp> {
    configured_harness(size, preferences, Default::default(), 1.0, false, "en")
}

fn configured_harness(
    size: egui::Vec2,
    preferences: bool,
    theme: crate::theme::ThemeKind,
    scale: f32,
    large: bool,
    language: &str,
) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("prefs.set", json!({"path": "interface.language", "value": language})).unwrap();
    if preferences {
        crate::prefs_ui::open_preferences(&mut app, "interface");
    } else {
        crate::prefs_ui::open_shortcuts(&mut app, 0);
    }
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(scale).build_ui_state(
        |ui, app| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                let _language = crate::i18n::language_scope(crate::i18n::Lang::from_pref(&app.session.prefs().interface.language));
                show(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    if large {
        crate::theme::set_ui_font_size(&h.ctx, photocraft_engine::prefs::UiFontSize::Large);
    }
    h.run_steps(5);
    h
}

fn frame(h: &Harness<'_, PhotocraftApp>) -> egui::Rect {
    rects(&h.ctx)[0]
}

fn assert_inside(h: &Harness<'_, PhotocraftApp>, label: &str, size: egui::Vec2) {
    let r = h.get_by_label(label).rect();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    assert!(screen.contains_rect(r), "{label}: {r:?} outside {screen:?}");
}

#[test]
fn shortcuts_width_is_stable_on_every_tab_switch_frame() {
    let mut h = harness(egui::vec2(1440.0, 900.0), false);
    let width = frame(&h).width();
    for tab in ["Menus", "Toolbar", "Keyboard Shortcuts", "Menus", "Keyboard Shortcuts"] {
        let pos = h.get_by_label(tab).rect().center();
        h.event(egui::Event::PointerMoved(pos));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
            assert!((frame(&h).width() - width).abs() < 0.5, "{tab}, pressed={pressed}: {:?}; initial width {width}", frame(&h));
        }
        for step in 0..5 {
            h.step();
            assert!((frame(&h).width() - width).abs() < 0.5, "{tab}, step {step}: {:?}; initial width {width}", frame(&h));
        }
    }
}

#[test]
fn preferences_and_shortcuts_keep_confirmation_buttons_in_a_small_window() {
    let size = egui::vec2(800.0, 600.0);
    for preferences in [false, true] {
        let h = harness(size, preferences);
        for label in ["OK", "Cancel"] {
            assert_inside(&h, label, size);
        }
        if preferences {
            assert_inside(&h, "Apply", size);
        }
    }
}

#[test]
fn eight_alternate_bindings_do_not_expand_the_shortcuts_dialog() {
    let size = egui::vec2(800.0, 600.0);
    let mut h = harness(size, false);
    let width = frame(&h).width();
    let d = h.state_mut().ui.dialogs.last_mut().unwrap();
    d.fields.insert("filter".into(), json!("Copy"));
    let keys: Vec<String> = "ABCDEFGH".chars().map(|c| format!("Cmd+Alt+Shift+{c}")).collect();
    d.fields.insert("alternates".into(), json!({"edit.copy": keys}));
    h.run_steps(5);
    assert!((frame(&h).width() - width).abs() < 0.5, "alternate keys expanded {:?} from {width}", frame(&h));
    assert!(frame(&h).right() <= size.x, "alternate keys extend past the window: {:?}", frame(&h));
    assert_inside(&h, "OK", size);
    assert_inside(&h, "Cancel", size);
    let binding = format!("{}  ×", crate::shortcuts::pretty("Cmd+Alt+Shift+A"));
    let top = h.get_by_label(&binding).rect().top();
    for step in 0..12 {
        h.step();
        assert!((h.get_by_label(&binding).rect().top() - top).abs() < 0.5, "alternate row kept growing at frame {step}");
    }
}

#[test]
fn menu_columns_do_not_shift_after_switching_from_shortcuts() {
    for (size, scale, large, language) in
        [(egui::vec2(1440.0, 900.0), 1.0, false, "en"), (egui::vec2(800.0, 600.0), 1.5, true, "ru"), (egui::vec2(800.0, 600.0), 2.0, true, "de")]
    {
        let mut h = configured_harness(size, false, Default::default(), scale, large, language);
        let width = frame(&h).width();
        let lang = crate::i18n::Lang::from_pref(language);
        let pos = h.get_by_label(crate::i18n::tr(lang, "Menus")).rect().center();
        h.event(egui::Event::PointerMoved(pos));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        let first = h.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().rect();
        for step in 0..5 {
            let current = h.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().rect();
            let visible = h.query_all_by_label(crate::i18n::tr(lang, "Visible")).next().unwrap().rect();
            let gap = current.left() - visible.right();
            assert!((gap - 12.0).abs() < 1.1, "{language}: checkbox is too far from its color selector: {gap}");
            assert!((current.left() - first.left()).abs() < 0.5, "Menus column shifted at {step}: first {first:?}, now {current:?}");
            assert!((frame(&h).width() - width).abs() < 0.5, "{language}: Menus changed the dialog width");
            h.step();
        }
    }
}

#[test]
fn dialog_actions_fit_themes_scales_and_large_translated_text() {
    for theme in crate::theme::ThemeKind::ALL {
        for (size, scale, large, language) in
            [(egui::vec2(800.0, 600.0), 1.0, false, "en"), (egui::vec2(800.0, 600.0), 1.5, true, "ru"), (egui::vec2(1440.0, 900.0), 2.0, true, "de")]
        {
            for preferences in [false, true] {
                let mut h = configured_harness(size, preferences, theme, scale, large, language);
                if !preferences {
                    let fields = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
                    fields.insert("filter".into(), json!("Copy"));
                    fields.insert("alternates".into(), json!({"edit.copy": ["Cmd+Alt+Shift+A", "Cmd+Alt+Shift+B", "Cmd+Alt+Shift+C", "Cmd+Alt+Shift+D", "Cmd+Alt+Shift+E", "Cmd+Alt+Shift+F", "Cmd+Alt+Shift+G", "Cmd+Alt+Shift+H"]}));
                    h.run_steps(5);
                }
                for label in ["OK", "Cancel"] {
                    assert_inside(&h, crate::i18n::tr(crate::i18n::Lang::from_pref(language), label), size);
                }
                assert!(frame(&h).width() <= size.x + 1.0, "{theme:?}/{language}: {:?}", frame(&h));
            }
        }
    }
}

#[test]
fn alternate_buttons_still_remove_and_capture_individual_bindings() {
    let mut h = harness(egui::vec2(800.0, 600.0), false);
    let d = h.state_mut().ui.dialogs.last_mut().unwrap();
    d.fields.insert("filter".into(), json!("edit.copy"));
    d.fields.insert("selected".into(), json!("edit.copy"));
    d.fields.insert("alternates".into(), json!({"edit.copy": ["F3", "Cmd+Alt+Shift+K"]}));
    h.run_steps(3);
    h.get_by_label("F3  ×").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["alternates"]["edit.copy"], json!(["Cmd+Alt+Shift+K"]));
    h.get_by_label("Add Shortcut").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["captureAlt"], true);
    h.event(egui::Event::Key { key: egui::Key::F8, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["alternates"]["edit.copy"], json!(["Cmd+Alt+Shift+K", "F8"]));
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["captureAlt"], false);
    h.get_by_label("Use Default").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.last().unwrap().fields["alternates"].get("edit.copy").is_none());
}

#[test]
fn protected_menu_items_cannot_be_hidden_by_the_layout_controls() {
    let mut h = harness(egui::vec2(800.0, 600.0), false);
    let d = h.state_mut().ui.dialogs.last_mut().unwrap();
    d.fields.insert("tab".into(), json!(1));
    d.fields.insert("filter".into(), json!("edit.keyboardShortcuts"));
    d.fields.insert("hidden".into(), json!(["edit.keyboardShortcuts"]));
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["hidden"], json!([]));
    h.get_by_label("Visible").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["hidden"], json!([]));
}

#[test]
fn shortcuts_restore_their_scroll_position_independently_of_menus() {
    let mut h = harness(egui::vec2(800.0, 600.0), false);
    let initial = h.get_by_label("   Open…").rect().top();
    h.get_by_label("   Open…").scroll_down();
    h.run_steps(3);
    let scrolled = h.get_by_label("   Open…").rect().top();
    assert!(scrolled < initial - 20.0, "scroll did not move the command row");
    h.get_by_label("Menus").click();
    h.run_steps(3);
    let menu_top = h.get_by_label("   Open…").rect().top();
    assert!(menu_top > scrolled + 20.0, "Menus inherited the shortcuts offset");
    h.get_by_label("Keyboard Shortcuts").click();
    h.run_steps(3);
    assert!((h.get_by_label("   Open…").rect().top() - scrolled).abs() < 0.5);
}

#[test]
fn conflict_actions_and_confirmation_remain_inside_the_dialog() {
    let size = egui::vec2(800.0, 600.0);
    for accept in [false, true] {
        let mut h = harness(size, false);
        let d = h.state_mut().ui.dialogs.last_mut().unwrap();
        d.fields.insert("filter".into(), json!("edit.copy"));
        d.fields.insert("selected".into(), json!("edit.copy"));
        d.fields.insert("capture".into(), json!(true));
        h.event(egui::Event::Key {
            key: egui::Key::O,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { command: true, ctrl: true, ..Default::default() },
        });
        h.run_steps(3);
        assert!(h.state().ui.dialogs.last().unwrap().fields["conflict"].is_object());
        for label in ["Accept and Go to Conflict", "Undo Changes", "OK", "Cancel"] {
            assert_inside(&h, label, size);
        }
        h.get_by_label(if accept { "Accept and Go to Conflict" } else { "Undo Changes" }).click();
        h.run_steps(3);
        let f = &h.state().ui.dialogs.last().unwrap().fields;
        assert!(f["conflict"].is_null());
        if accept {
            assert_eq!(f["overrides"]["file.open"], "");
            assert_eq!(f["selected"], "file.open");
            assert_eq!(f["filter"], "");
        } else {
            assert!(f["overrides"].get("edit.copy").is_none());
        }
    }
}

#[test]
fn long_shortcut_set_names_can_be_created_and_saved_without_expanding_the_dialog() {
    let size = egui::vec2(800.0, 600.0);
    let mut h = harness(size, false);
    let width = frame(&h).width();
    let name = "W".repeat(64);
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("newSetName".into(), json!(name));
    h.run_steps(3);
    assert_inside(&h, "Create", size);
    h.get_by_label("Create").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["activeSet"], name);
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("overrides".into(), json!({"edit.copy": "F9"}));
    h.run_steps(3);
    h.get_by_label("Save").click();
    h.run_steps(3);
    let f = &h.state().ui.dialogs.last().unwrap().fields;
    assert_eq!(f["sets"][&name]["shortcuts"]["edit.copy"], "F9");
    assert!((frame(&h).width() - width).abs() < 0.5);
    assert_inside(&h, "OK", size);
}

#[test]
fn dialog_buttons_stay_visible_after_the_viewport_is_shrunk() {
    let size = egui::vec2(800.0, 600.0);
    for preferences in [false, true] {
        let mut h = harness(egui::vec2(1440.0, 900.0), preferences);
        h.set_size(size);
        h.run_steps(4);
        for label in ["OK", "Cancel"] {
            assert_inside(&h, label, size);
        }
    }
}

#[test]
fn measurement_log_columns_do_not_reuse_another_schema_width() {
    use photocraft_engine::analysis_cmds::LogRow;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.analysis.measurement_log = true;
    app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
    app.session.analysis.log.push(LogRow { id: 1, values: json!({"label": "W".repeat(160), "count": 1}).as_object().unwrap().clone() });
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                crate::analysis_ui::windows(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h.state_mut().session.analysis.log = vec![LogRow { id: 2, values: json!({"length": 20, "angle": 45}).as_object().unwrap().clone() }];
    h.step();
    let first = h.query_by_label("Angle").map(|node| node.rect());
    h.run_steps(3);
    let current = h.get_by_label("Angle").rect();
    if let Some(first) = first {
        assert!((first.left() - current.left()).abs() < 0.5, "stale column width: {first:?} -> {current:?}");
    }
    assert!(current.right() <= 1440.0);
    h.state_mut().session.analysis.log.clear();
    h.run_steps(3);
    assert!(h.query_by_label("Angle").is_none());
}
