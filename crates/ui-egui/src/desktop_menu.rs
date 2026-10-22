//! Linux desktop menu snapshot. Reuses local menu ordering, state, translations and dispatch;
//! it never changes local-menu visibility or the macOS backend.

use craft_desktop::{Action, Item, Menu};
use serde_json::{Value, json};

use crate::{PhotocraftApp, menus, native_menu};

/// Reuse the native bar's cheap dirty-state fingerprint instead of walking hundreds of
/// command predicates during every canvas animation frame.
pub fn state_key(app: &PhotocraftApp, ctx: &egui::Context) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    native_menu::state_hash(app, native_menu::had_input(ctx).then_some(app.frame)).hash(&mut hash);
    app.file_dialog_open().hash(&mut hash);
    hash.finish()
}

fn target_identity(app: &PhotocraftApp, id: &str) -> Value {
    if let Some(index) = id.strip_prefix("file.openRecent.").and_then(|s| s.parse::<usize>().ok()) {
        return json!({"recent": app.ui.recent_files.get(index)});
    }
    if let Some(index) = id.strip_prefix(menus::DOCUMENT_ITEM).and_then(|s| s.parse::<usize>().ok()) {
        return json!({"document": app.session.documents().get(index).map(|s| s.doc.id.0)});
    }
    Value::Null
}

pub fn snapshot(app: &PhotocraftApp) -> Menu {
    let lang = crate::i18n::Lang::from_pref(&app.session.prefs().interface.language);
    let mut rows = menus::menu_items(app);
    for row in &mut rows {
        row.enabled &= !app.file_dialog_open() && menus::modal_allows(app, &row.id);
        // File names are data, not translation keys; command labels use the local bar's
        // dynamic quick-export and command-context translation rules.
        if !row.id.starts_with("file.openRecent.") {
            row.label = menus::translated_menu_label(lang, row);
        }
    }
    let bar = native_menu::from_items(&menus::TOP_MENUS, &rows);
    fn level(app: &PhotocraftApp, nodes: &[native_menu::Node], lang: crate::i18n::Lang) -> Vec<Item> {
        nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                Some(match node {
                    native_menu::Node::Item(row) => Item {
                        key: row.id.clone(),
                        label: row.label.clone(),
                        enabled: row.enabled,
                        checked: row.checked,
                        shortcut: row.shortcut.as_deref().unwrap_or_default().replace("Cmd", "Ctrl"),
                        action: Some(Action { command: row.id.clone(), params: target_identity(app, &row.id) }),
                        ..Default::default()
                    },
                    native_menu::Node::Separator => Item { key: format!("separator-{index}"), separator: true, ..Default::default() },
                    native_menu::Node::Submenu { label, children, .. } => Item {
                        key: label.clone(),
                        label: crate::i18n::tr(lang, label).into(),
                        enabled: true,
                        children: level(app, children, lang),
                        ..Default::default()
                    },
                    native_menu::Node::Standard(_) => return None,
                })
            })
            .collect()
    }
    Menu {
        document: app.session.active().map_or(0, |doc| doc.doc.id.0),
        items: bar
            .menus
            .iter()
            .map(|menu| Item {
                key: menu.title.clone(),
                label: crate::i18n::tr(lang, &menu.title).into(),
                enabled: true,
                children: level(app, &menu.children, lang),
                ..Default::default()
            })
            .collect(),
    }
}

fn contains_enabled(items: &[Item], action: &Action) -> bool {
    items.iter().any(|item| item.enabled && (item.action.as_ref() == Some(action) || contains_enabled(&item.children, action)))
}

/// Reject old documents, changed recent/document slots, removed commands and newly opened
/// modals. These are user menu clicks, so use the interactive dispatcher and its save guards.
pub fn activate(app: &mut PhotocraftApp, ctx: &egui::Context, document: u64, action: &Action) -> Result<(), String> {
    let current = snapshot(app);
    if current.document != document || !contains_enabled(&current.items, action) {
        return Ok(());
    }
    menus::invoke(app, ctx, &action.command, json!({})).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    }

    fn find<'a>(items: &'a [Item], command: &str) -> Option<&'a Item> {
        items.iter().find_map(|item| if item.action.as_ref().is_some_and(|a| a.command == command) { Some(item) } else { find(&item.children, command) })
    }

    #[test]
    fn desktop_menu_uses_local_sections_and_interactive_new_dialog() {
        let mut app = app();
        let menu = snapshot(&app);
        assert_eq!(menu.items.len(), menus::TOP_MENUS.len());
        let new = find(&menu.items, "file.new").unwrap().action.as_ref().unwrap();
        activate(&mut app, &egui::Context::default(), menu.document, new).unwrap();
        assert!(!app.ui.dialogs.is_empty(), "New opens its normal interactive dialog");
        assert!(app.services.native_menu.is_none(), "local menus stay available");
    }

    #[test]
    fn desktop_menu_drops_changed_recent_targets_and_old_documents() {
        let mut app = app();
        app.ui.recent_files = vec!["/tmp/a/same.png".into()];
        let before = snapshot(&app);
        let recent = find(&before.items, "file.openRecent.0").unwrap().action.clone().unwrap();
        app.ui.recent_files = vec!["/tmp/b/same.png".into()];
        assert!(!contains_enabled(&snapshot(&app).items, &recent));
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let new = find(&before.items, "file.new").unwrap().action.clone().unwrap();
        activate(&mut app, &egui::Context::default(), before.document, &new).unwrap();
        assert!(app.ui.dialogs.is_empty());
    }

    #[test]
    fn desktop_menu_tracks_undo_state_and_modal_guards() {
        let mut app = app();
        assert!(!find(&snapshot(&app).items, "edit.undo").unwrap().enabled);
        let current = snapshot(&app);
        let new = find(&current.items, "file.new").unwrap().action.clone().unwrap();
        activate(&mut app, &egui::Context::default(), current.document, &new).unwrap();
        assert!(!find(&snapshot(&app).items, "file.new").unwrap().enabled);
    }

    #[test]
    fn desktop_menu_checkmarks_follow_the_shared_view_command() {
        let mut app = app();
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.sync_views();
        let before = snapshot(&app);
        let row = find(&before.items, "view.rulers").unwrap();
        let checked = row.checked.unwrap();
        activate(&mut app, &egui::Context::default(), before.document, row.action.as_ref().unwrap()).unwrap();
        assert_eq!(find(&snapshot(&app).items, "view.rulers").unwrap().checked, Some(!checked));
    }
}
