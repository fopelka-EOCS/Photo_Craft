//! The first-launch setup: a one-time window that asks who is setting PhotoCraft up before
//! they open anything.
//!
//! It asks the two questions whose answers change everything that follows — the theme the app is
//! drawn in, and the language its UI is in — and it asks them once. Preferences › `dialogs`
//! records the answer ([`DONE_KEY`]), so a later launch goes straight to the Home screen and
//! nobody is asked again. Skipping is a real answer: the defaults stay and the flag is set the
//! same way, because a first-run screen that reappears is worse than one that guessed.

use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Preferences › `dialogs` key recording that the first-launch setup has been answered.
pub const DONE_KEY: &str = "firstLaunchSetup";

/// Open the setup once, the first time the app runs.
pub fn offer(app: &mut PhotocraftApp) {
    // A one-time question needs somewhere to keep the answer. Without preference storage (the
    // test harnesses and embedded uses), asking again on every launch would be worse than guessing.
    if app.services.load_prefs.is_none() || app.services.save_prefs.is_none() {
        return;
    }
    if app.session.prefs().dialogs.contains_key(DONE_KEY) {
        return;
    }
    app.ui.first_launch_setup = true;
}

fn remember(app: &mut PhotocraftApp) {
    app.session.prefs.edit(|p| {
        p.dialogs.insert(DONE_KEY.into(), Value::Bool(true));
    });
}

/// Whether the setup window belongs on screen: offered, and not over a document.
pub(crate) fn visible(app: &PhotocraftApp) -> bool {
    app.ui.first_launch_setup && app.session.documents().is_empty()
}

/// The window. Closes once the person has answered, whether they chose a theme or skipped.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // On the Home screen only: someone who opened a document before answering is not interrupted
    // by a setup window over their work, and the canvas keeps the pointer it is given.
    if !visible(app) {
        return;
    }
    let mut done = None;
    let mut theme = app.ui.theme;
    let mut language = app.session.prefs().interface.language.clone();

    let open = egui::Window::new(tl!("Welcome to PhotoCraft"))
        .id(egui::Id::new("first-launch-setup"))
        .collapsible(false)
        .resizable(false)
        .default_width(470.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            let t = crate::theme::Tokens::get(ui.ctx());
            let inner = ui.available_width();
            ui.label(egui::RichText::new(tl!("Let's start something new.")).font(crate::theme::semibold(19.0)).color(t.text));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(tl!("Choose a theme and a language. Both can be changed later in Settings.")).color(t.text_dim).size(12.5));
            ui.add_space(16.0);

            // The theme: the fourteen the app ships, each named the way the list names it.
            ui.label(egui::RichText::new(tl!("Theme")).font(crate::theme::semibold(12.0)).color(t.text));
            ui.add_space(4.0);
            let themes: Vec<(crate::theme::ThemeKind, &str)> = crate::theme::ThemeKind::ALL.iter().map(|k| (*k, k.label())).collect();
            if crate::widgets::dropdown(ui, "first-launch-theme", &mut theme, &themes, inner) {
                app.ui.theme = theme;
            }

            ui.add_space(12.0);
            // The language: the same list Preferences › Interface offers, with "Auto" first.
            ui.label(egui::RichText::new(tl!("Language")).font(crate::theme::semibold(12.0)).color(t.text));
            ui.add_space(4.0);
            let mut pairs: Vec<(String, &str)> = vec![("auto".into(), tl!("Auto"))];
            pairs.extend(crate::i18n::Lang::all().map(|l| (l.code().to_string(), l.name())));
            if crate::widgets::dropdown(ui, "first-launch-language", &mut language, &pairs, inner) {
                // The preview follows the selection, as it does in Preferences › Interface.
                app.session.prefs.edit(|p| p.interface.language = language.clone());
                crate::i18n::sync_context(ui.ctx(), &language);
            }

            ui.add_space(18.0);
            use crate::widgets::{ButtonRole, DialogButton};
            let buttons = [DialogButton::new(ButtonRole::Default, tl!("Start"), 110.0), DialogButton::new(ButtonRole::Alternate, tl!("Skip"), 90.0)];
            if let Some(role) = crate::widgets::dialog_buttons(ui, &buttons) {
                done = Some(role);
            }
        });
    if open.is_none() {
        return;
    }

    let Some(role) = done else { return };
    if role == crate::widgets::ButtonRole::Default {
        // `prefs.set` is the command the rest of the shell uses for a preference, so the first
        // launch applies its choices through the same path and they validate the same way.
        if let Err(e) = app.run("prefs.set", json!({ "values": { "interface.theme": theme.id(), "interface.language": language } })) {
            app.ui.status = crate::i18n::fmt(tl!("Could not apply the setup: {error}"), &[("error", &e)]);
            app.ui.status_error = true;
            return;
        }
        app.ui.theme = theme;
    }
    remember(app);
    app.ui.first_launch_setup = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setup_needs_somewhere_to_remember_the_answer() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.first_launch_setup = false;
        offer(&mut app);
        assert!(!app.ui.first_launch_setup, "no preference storage, no one-time question");
    }

    #[test]
    fn the_setup_window_waits_for_the_home_screen() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.first_launch_setup = true;
        assert!(visible(&app));
        app.session.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
        assert!(!visible(&app), "an open document keeps the setup window off the canvas");
    }
}
