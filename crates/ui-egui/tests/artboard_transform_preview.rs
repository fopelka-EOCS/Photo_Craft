//! #7944: Free Transform on a type or shape layer inside an artboard. While the box is dragged,
//! the other layers in the board must stay visible and the board must not turn blank white.
//!
//! Drives the real app offscreen on wgpu: a white artboard on a black canvas holds a blue shape,
//! a yellow type layer, a red shape and a green shape over it. Each test opens Free Transform on
//! one layer, drags the box with real pointer events, and checks the pixels before the drag,
//! while the button is held, and after Enter. Set `PHOTOCRAFT_ARTBOARD_TRANSFORM_EVIDENCE_DIR`
//! to also save those frames as PNGs. Skips when no GPU adapter exists.

use egui::{Modifiers, PointerButton};
use photocraft_doc::LayerId;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::canvas::ViewXform;
use photocraft_ui_egui::control::{ControlRequest, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

fn harness() -> Option<Harness> {
    let built = std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 640.0)).with_max_steps(64).wgpu().build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                app.set_wgpu(rs.clone());
            }
            app
        })
    });
    built.map_err(|_| eprintln!("skipping: no GPU adapter")).ok()
}

fn control(h: &mut Harness, method: &str, params: Value) {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params);
    handle(h.state_mut(), &ctx, &req);
    h.run_steps(3);
}

fn run(h: &mut Harness, command: &str, params: Value) -> Value {
    h.state_mut().run(command, params).unwrap_or_else(|e| panic!("{command}: {e}"))
}

fn layer(v: &Value) -> LayerId {
    LayerId(v["layer"].as_u64().expect("a layer id"))
}

/// The board and its layers: (red shape, yellow type).
fn setup(h: &mut Harness) -> (LayerId, LayerId) {
    run(h, "prefs.set", json!({"values": {"interface.language": "en"}}));
    run(h, "file.new", json!({"width": 400, "height": 300, "background": "black"}));
    let blue = layer(&run(h, "shape.create", json!({"kind": "rect", "rect": [80, 80, 80, 80], "fill": "#0000ff"})));
    let text = layer(&run(h, "type.create", json!({"x": 150, "y": 240, "text": "Hi", "size": 80, "color": "#ffff00"})));
    let red = layer(&run(h, "shape.create", json!({"kind": "rect", "rect": [180, 100, 60, 60], "fill": "#ff0000"})));
    let green = layer(&run(h, "shape.create", json!({"kind": "rect", "rect": [90, 180, 40, 40], "fill": "#00ff00"})));
    run(h, "layer.select", json!({"layer": blue.0}));
    for id in [text, red, green] {
        run(h, "layer.select", json!({"layer": id.0, "mode": "add"}));
    }
    run(h, "layer.new.artboardFromLayers", json!({"name": "Board"}));
    run(h, "layer.artboard.set", json!({"width": 220, "height": 180}));
    h.state_mut().sync_views();
    h.state_mut().ui.extras.rulers = false;
    control(h, "ui.set", json!({"tool": "move", "zoom": 1.0, "center": [200, 150]}));
    h.run_steps(3);
    (red, text)
}

fn xf(h: &Harness) -> ViewXform {
    ViewXform::active(h.state()).expect("an active document view")
}

fn pointer(h: &mut Harness, doc: [f32; 2], pressed: Option<bool>) {
    let p = xf(h).to_screen(doc[0], doc[1]);
    h.event(egui::Event::PointerMoved(p));
    if let Some(pressed) = pressed {
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.step();
}

/// A rendered frame: RGBA8, physical pixels.
struct Frame {
    w: u32,
    px: Vec<u8>,
}

impl Frame {
    /// RGB at document point `(x, y)`.
    fn rgb(&self, h: &Harness, x: f32, y: f32) -> [u8; 3] {
        let p = xf(h).to_screen(x, y);
        let i = (p.y.round() as usize * self.w as usize + p.x.round() as usize) * 4;
        [self.px[i], self.px[i + 1], self.px[i + 2]]
    }
}

/// The current frame, saved as `name` when evidence is asked for.
fn frame(h: &mut Harness, name: &str) -> Frame {
    h.run_steps(3);
    let img = h.render().expect("render");
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_ARTBOARD_TRANSFORM_EVIDENCE_DIR") {
        std::fs::create_dir_all(&dir).expect("evidence directory");
        img.save(std::path::Path::new(&dir).join(format!("{name}.png"))).expect("save frame");
    }
    Frame { w: img.width(), px: img.into_raw() }
}

fn is(c: [u8; 3], want: [u8; 3]) -> bool {
    c.iter().zip(want).all(|(a, b)| a.abs_diff(b) < 40)
}

const BLUE: [u8; 3] = [0, 0, 255];
const GREEN: [u8; 3] = [0, 255, 0];
const RED: [u8; 3] = [255, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

/// The layers that don't move, the board's own white and the pasteboard around it. The points
/// stay clear of the smart guides and the drag's ΔX/ΔY readout.
fn assert_board(h: &Harness, f: &Frame, when: &str) {
    assert!(is(f.rgb(h, 95.0, 95.0), BLUE), "{when}: the blue shape shows, got {:?}", f.rgb(h, 95.0, 95.0));
    assert!(is(f.rgb(h, 100.0, 190.0), GREEN), "{when}: the green shape shows, got {:?}", f.rgb(h, 100.0, 190.0));
    assert!(is(f.rgb(h, 285.0, 95.0), WHITE), "{when}: the board's background shows, got {:?}", f.rgb(h, 285.0, 95.0));
    assert!(!is(f.rgb(h, 40.0, 40.0), WHITE), "{when}: the pasteboard outside the board, got {:?}", f.rgb(h, 40.0, 40.0));
}

/// Free Transform on `id`, a drag from `from` by (20, 20) held, then Enter. `from` must be inside
/// the box but off its centre: the reference point there moves only the pivot.
fn transform(h: &mut Harness, id: LayerId, from: [f32; 2], name: &str) -> (Frame, Frame, Frame) {
    run(h, "layer.select", json!({"layer": id.0}));
    control(h, "ui.menu.invoke", json!({"id": "edit.freeTransform"}));
    let before = frame(h, &format!("{name}-1-before"));
    pointer(h, from, None);
    pointer(h, from, Some(true));
    for i in 1..=6 {
        let d = 20.0 * i as f32 / 6.0;
        pointer(h, [from[0] + d, from[1] + d], None);
    }
    let during = frame(h, &format!("{name}-2-during"));
    pointer(h, [from[0] + 20.0, from[1] + 20.0], Some(false));
    h.key_press(egui::Key::Enter);
    let after = frame(h, &format!("{name}-3-after"));
    (before, during, after)
}

#[test]
fn transforming_a_shape_in_an_artboard_keeps_the_board_in_view() {
    let Some(mut h) = harness() else { return };
    let (red, _) = setup(&mut h);
    let (before, during, after) = transform(&mut h, red, [192.0, 112.0], "shape");
    for (f, when) in [(&before, "before"), (&during, "while dragging"), (&after, "after Enter")] {
        assert_board(&h, f, when);
    }
    assert!(is(before.rgb(&h, 190.0, 110.0), RED));
    // Right of the shape before the drag, inside it once moved by (20, 20).
    assert!(!is(before.rgb(&h, 245.0, 125.0), RED));
    assert!(is(during.rgb(&h, 245.0, 125.0), RED), "the moving shape shows where it is dragged, got {:?}", during.rgb(&h, 245.0, 125.0));
    assert!(is(after.rgb(&h, 245.0, 125.0), RED), "committed where it was dropped");
}

#[test]
fn transforming_type_in_an_artboard_keeps_the_other_layers_in_view() {
    let Some(mut h) = harness() else { return };
    let (_, text) = setup(&mut h);
    let b = photocraft_engine::layer_multi_cmds::layer_bounds(h.state().session.active().unwrap().doc.layer(text).unwrap()).expect("type bounds");
    // Clear of the handles and of the reference point at the centre.
    let off_centre = [b.x0 as f32 + b.width() as f32 * 0.3, b.y0 as f32 + b.height() as f32 * 0.3];
    let (_, during, after) = transform(&mut h, text, off_centre, "type");
    for (f, when) in [(&during, "while dragging"), (&after, "after Enter")] {
        assert_board(&h, f, when);
        assert!(is(f.rgb(&h, 230.0, 110.0), RED), "{when}: the red shape shows, got {:?}", f.rgb(&h, 230.0, 110.0));
    }
    let moved = photocraft_engine::layer_multi_cmds::layer_bounds(h.state().session.active().unwrap().doc.layer(text).unwrap()).expect("type bounds");
    // About (20, 20): smart guides may snap the drag by a pixel or two.
    let (dx, dy) = (moved.x0 - b.x0, moved.y0 - b.y0);
    assert!((15..=25).contains(&dx) && (15..=25).contains(&dy), "the type moved, by ({dx}, {dy})");
    assert_eq!((moved.width(), moved.height()), (b.width(), b.height()), "unscaled");
}
