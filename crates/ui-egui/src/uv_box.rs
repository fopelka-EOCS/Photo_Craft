//! Rectangle shell picking, evaluated once on release, independent of texture resolution.
use crate::{PhotocraftApp, theme::Tokens};
use egui::{Pos2, Rect};

#[derive(Clone, Copy)]
pub(crate) struct Drag {
    start: Pos2,
    modifiers: egui::Modifiers,
}
pub(crate) fn input(ui: &egui::Ui, response: &egui::Response, drag: &mut Option<Drag>) -> Option<(Rect, egui::Modifiers)> {
    if ui.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
        *drag = None;
        return None;
    }
    if response.contains_pointer()
        && ui.input(|i| i.pointer.primary_pressed() && !i.modifiers.alt)
        && let Some(start) = ui.input(|i| i.pointer.interact_pos())
    {
        *drag = Some(Drag { start, modifiers: ui.input(|i| i.modifiers) });
    }
    let d = (*drag)?;
    let end = ui.input(|i| i.pointer.interact_pos()).unwrap_or(d.start);
    let rect = Rect::from_two_pos(d.start, end);
    if rect.size().length() > 4.0 {
        let t = Tokens::get(ui.ctx());
        ui.painter_at(response.rect).rect(rect, 0, t.accent.gamma_multiply(0.12), egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    if ui.input(|i| i.pointer.primary_released()) {
        *drag = None;
        return (rect.size().length() > 4.0).then_some((rect, d.modifiers));
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        *drag = None;
    }
    None
}

fn triangle_touches(rect: Rect, tri: [Pos2; 3]) -> bool {
    if tri.iter().any(|p| rect.contains(*p)) {
        return true;
    }
    if !Rect::from_points(&tri).intersects(rect) {
        return false;
    }
    // Clip against the rectangle: includes edges crossing it and a rectangle inside a triangle.
    let mut poly = tri.to_vec();
    for (axis, edge, sign) in [(0, rect.left(), -1.0), (0, rect.right(), 1.0), (1, rect.top(), -1.0), (1, rect.bottom(), 1.0)] {
        let mut out = Vec::new();
        for (a, b) in poly.iter().zip(poly.iter().cycle().skip(1)).take(poly.len()) {
            let value = |p: Pos2| ((if axis == 0 { p.x } else { p.y }) - edge) * sign;
            let (va, vb) = (value(*a), value(*b));
            if va <= 0.0 {
                out.push(*a);
            }
            if (va <= 0.0) != (vb <= 0.0) {
                out.push(a.lerp(*b, va / (va - vb)));
            }
        }
        poly = out;
    }
    !poly.is_empty()
}
pub(crate) fn select(
    app: &mut PhotocraftApp,
    state: &mut crate::uv_ui::State,
    rect: Rect,
    modifiers: egui::Modifiers,
    material: usize,
    project: impl Fn(usize) -> Option<[Pos2; 3]>,
) {
    let Some(topology) = app.session.uv.topology.clone() else { return };
    let shells: Vec<_> = topology
        .shells
        .iter()
        .enumerate()
        .filter(|(_, sh)| sh.material == material)
        .filter_map(|(id, sh)| {
            let selected = if state.shell_enclosed {
                sh.triangles.iter().all(|id| project(*id).is_some_and(|tri| tri.iter().all(|p| rect.contains(*p))))
            } else {
                sh.triangles.iter().any(|id| project(*id).is_some_and(|tri| triangle_touches(rect, tri)))
            };
            selected.then_some(id)
        })
        .collect();
    let mode = if modifiers.ctrl || modifiers.alt {
        "subtract"
    } else if modifiers.shift {
        "add"
    } else {
        "replace"
    };
    match app.run("uv.shell.select", serde_json::json!({"material":material,"shells":shells,"mode":mode})) {
        Ok(_) => {
            if let Err(e) = app.run("uv.mask.apply", serde_json::json!({"guard":state.guard,"horizontal":state.horizontal,"vertical":state.vertical,"feather":state.feather,"openBorders":state.open_borders})) {
                state.status = e;
            }
        }
        Err(e) => state.status = e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_selects_touched_or_enclosed_shells_and_supports_subtraction() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
        app.run("uv.model.import", serde_json::json!({"path":path})).unwrap();
        let topology = app.session.uv.topology.clone().unwrap();
        let material = app.session.uv.material;
        let project = |id| {
            if topology.triangle_shell[id] == 0 {
                Some([egui::pos2(-2.0, 2.0), egui::pos2(2.0, 2.0), egui::pos2(2.0, 8.0)])
            } else {
                Some([egui::pos2(4.0, 4.0), egui::pos2(6.0, 4.0), egui::pos2(6.0, 8.0)])
            }
        };
        let rect = Rect::from_min_max(Pos2::ZERO, egui::pos2(10.0, 10.0));
        let mut state = crate::uv_ui::State::default();
        select(&mut app, &mut state, rect, egui::Modifiers::NONE, material, project);
        assert_eq!(app.session.uv.selected.len(), 2);
        state.shell_enclosed = true;
        select(&mut app, &mut state, rect, egui::Modifiers::NONE, material, project);
        assert_eq!(app.session.uv.selected, std::collections::BTreeSet::from([1]));
        select(&mut app, &mut state, rect, egui::Modifiers { ctrl: true, ..Default::default() }, material, project);
        assert!(app.session.uv.selected.is_empty());
    }
    #[test]
    fn touches_includes_crossing_edges_and_rect_inside_but_rejects_bounding_box_false_positive() {
        let _gpu = photocraft_testkit::gpu_lock();
        let tri = [egui::pos2(0.0, 0.0), egui::pos2(100.0, 0.0), egui::pos2(0.0, 100.0)];
        let r = |a, b, c, d| Rect::from_min_max(egui::pos2(a, b), egui::pos2(c, d));
        assert!(triangle_touches(r(10.0, 10.0, 20.0, 20.0), tri));
        assert!(triangle_touches(r(40.0, -1.0, 60.0, 1.0), tri));
        assert!(!triangle_touches(r(80.0, 80.0, 90.0, 90.0), tri));
    }
}
