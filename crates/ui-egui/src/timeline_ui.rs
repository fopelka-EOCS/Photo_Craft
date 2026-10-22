//! Window › Timeline: a floating panel that creates and drives the document timeline. All actions
//! run the engine `timeline.*` commands, so the control channel drives it the same way.

use egui::{RichText, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Timeline panel view state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimelineUi {
    pub open: bool,
    /// Playing back (auto-advancing the playhead at the frame rate).
    #[serde(default)]
    pub playing: bool,
    /// egui time (seconds) of the last auto-advance.
    #[serde(default, skip)]
    pub last_time: f64,
    #[serde(default)]
    pub onion_skin: bool,
    #[serde(default)]
    pub reverse_layers: bool,
}

/// Layer hierarchy shared by the track panel and its tests. Group children stay visible.
fn tracks(layers: &[photocraft_doc::Layer], depth: usize, out: &mut Vec<(u64, String, usize, usize)>) {
    if depth > 64 {
        return;
    }
    for layer in layers {
        out.push((layer.id.0, layer.name.clone(), layer.video.as_ref().map_or(0, |v| v.frames.len()), depth));
        if let Some(children) = layer.children() {
            tracks(children, depth + 1, out);
        }
    }
}

/// Menu checkmark for Window › Timeline.
pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    (id == "window.panel.timeline").then_some(app.ui.timeline.open)
}

/// Ids this module owns (for parity / enablement).
pub fn handles(id: &str) -> bool {
    id == "window.panel.timeline"
}

/// Toggle the Timeline panel.
pub fn menu(app: &mut PhotocraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    if id == "window.panel.timeline" {
        app.ui.timeline.open = !app.ui.timeline.open;
        return Some(Ok(json!({ "open": app.ui.timeline.open })));
    }
    None
}

/// Render the Timeline panel.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !crate::panel_docking::managed(app, "timeline") {
        render(app, ctx, None);
    }
}
pub(crate) fn dock_body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    render(app, &ctx, Some(ui));
}
fn render(app: &mut PhotocraftApp, ctx: &egui::Context, inline: Option<&mut egui::Ui>) {
    if !app.ui.timeline.open {
        return;
    }
    // Saving/exporting owns a document snapshot. Playback must not invalidate it.
    if app.session.active_job().is_some() {
        app.ui.timeline.playing = false;
    }
    let tl = app.session.active().and_then(|d| d.doc.timeline.clone());
    let have_doc = app.session.active().is_some();
    let mut act: Option<(&str, Value)> = None;
    let mut select_layer = None;
    let mut close = false;
    let mut toggle_play = false;
    let mut stop_play = false;
    let playing = app.ui.timeline.playing;
    let mut onion = app.ui.timeline.onion_skin;
    let mut reverse_layers = app.ui.timeline.reverse_layers;
    let mut rows = Vec::new();
    let active = app.session.active().and_then(|d| d.active_layer).map(|id| id.0);
    if let Some(d) = app.session.active() {
        tracks(&d.doc.layers, 0, &mut rows);
    }
    rows.sort_by_key(|(_, _, count, _)| *count == 0);
    let active_video = rows.iter().any(|(id, _, n, _)| Some(*id) == active && *n > 0);
    let t = Tokens::get(ctx);

    let docked = inline.is_some();
    let mut contents = |ui: &mut egui::Ui| {
        if !docked {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Timeline")).strong().color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::button(ui, "x", 20.0, false, tl!("Close")).clicked() {
                        close = true;
                    }
                });
            });
            ui.separator();
        }

        if !have_doc {
            ui.weak(tl!("Open a document to use the timeline."));
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.menu_button("从图层创建帧", |ui| {
                ui.label("一层一帧；保留并隐藏原图层，背景不参与。");
                ui.checkbox(&mut reverse_layers, "倒序（从最上层开始）");
                for (label, scope) in [("所选图层 → 帧动画", "selected"), ("全部顶层图层 → 帧动画", "all")] {
                    if ui.button(label).clicked() {
                        stop_play = true;
                        act = Some(("layer.videoLayers.fromLayers", json!({"scope":scope,"reverse":reverse_layers})));
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("默认从底层向顶层排列；组作为一帧。原动画时间轴将替换，支持撤销。");
        });
        match &tl {
            None => {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    if crate::widgets::primary_button(ui, tl!("Create Video Timeline"), 200.0).clicked() {
                        act = Some(("timeline.create", json!({ "duration": 12, "fps": 12.0 })));
                    }
                    ui.add_space(4.0);
                    ui.weak("12 帧 · 12 fps；创建后添加逐帧图层开始绘制");
                });
            }
            Some(tl) => {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("新建逐帧图层").clicked() {
                        act = Some(("layer.videoLayers.newBlankVideoLayer", json!({})));
                    }
                    ui.add_enabled_ui(active_video, |ui| {
                        if ui.button("插入空白帧").clicked() {
                            stop_play = true;
                            act = Some(("layer.videoLayers.insertBlankFrame", json!({})));
                        }
                        if ui.button("复制当前帧").clicked() {
                            stop_play = true;
                            act = Some(("layer.videoLayers.duplicateFrame", json!({})));
                        }
                        if ui.button("删除当前帧").clicked() {
                            stop_play = true;
                            act = Some(("layer.videoLayers.deleteFrame", json!({})));
                        }
                    });
                    ui.checkbox(&mut onion, "洋葱皮（前红 / 后绿）");
                });
                ui.horizontal(|ui| {
                    let mut fps = tl.fps;
                    ui.label("帧率");
                    if ui.add(egui::DragValue::new(&mut fps).range(1.0..=120.0).suffix(" fps")).changed() {
                        act = Some(("timeline.setProps", json!({"fps":fps})));
                    }
                    let mut duration = tl.duration;
                    ui.label("总帧数");
                    if ui.add(egui::DragValue::new(&mut duration).range(1..=10000)).changed() {
                        stop_play = true;
                        act = Some(("timeline.setProps", json!({"duration":duration})));
                    }
                });
                if !rows.iter().any(|(_, _, n, _)| *n > 0) {
                    ui.colored_label(t.text_dim, "已有画稿：点击「从图层创建帧」；空白绘制：点击「新建逐帧图层」。");
                } else if !active_video {
                    ui.weak("当前选中静态图层；请点击 [逐帧] 图层，再在各帧绘制。");
                } else if let Some((_, name, count, _)) = rows.iter().find(|(id, _, _, _)| Some(*id) == active) {
                    ui.label(format!("当前绘制：{name} · 第 {} 帧 / {count} 帧", tl.current.min(count.saturating_sub(1)) + 1));
                }
                let first = tl.current.saturating_sub(8);
                let last = first.saturating_add(24).min(tl.duration);
                let name_width = (ui.available_width() * 0.22).clamp(120.0, 220.0);
                let cell_width = ((ui.available_width() - name_width - 20.0) / (last - first).max(1) as f32 - 3.0).max(26.0);
                let track_height = if docked { (ui.available_height() - 108.0).max(44.0) } else { 200.0 };
                egui::ScrollArea::both().id_salt("timeline-tracks").auto_shrink([false, false]).max_height(track_height).show(ui, |ui| {
                    egui::Grid::new("timeline-layer-frames").spacing(vec2(3.0, 3.0)).show(ui, |ui| {
                        ui.add_sized([name_width, 20.0], egui::Label::new("图层 / 帧"));
                        // Virtualise the horizontal frame range around the playhead.
                        for frame in first..last {
                            if ui.add_sized([cell_width, 20.0], egui::Button::selectable(frame == tl.current, (frame + 1).to_string())).clicked() {
                                stop_play = true;
                                act = Some(("timeline.setFrame", json!({"frame":frame})));
                            }
                        }
                        ui.end_row();
                        for (id, name, count, depth) in &rows {
                            let name = format!("{}{}{}", "  ".repeat(*depth), name, if *count == 0 { " [静态]" } else { " [逐帧]" });
                            if ui.add_sized([name_width, 20.0], egui::Button::selectable(Some(*id) == active, name).truncate()).clicked() {
                                stop_play = true;
                                act = Some(("layer.select", json!({"layer":id})));
                            }
                            for frame in first..last {
                                let label = if *count == 0 {
                                    "—"
                                } else if frame < *count {
                                    "●"
                                } else {
                                    "↳"
                                };
                                if ui
                                    .add_sized([cell_width, 20.0], egui::Button::selectable(frame == tl.current && Some(*id) == active, label))
                                    .on_hover_text(format!("第 {} 帧；{} 帧内容", frame + 1, count))
                                    .clicked()
                                {
                                    stop_play = true;
                                    select_layer = Some(*id);
                                    act = Some(("timeline.setFrame", json!({"frame":frame})));
                                }
                            }
                            ui.end_row();
                        }
                    });
                });
                ui.label(egui::RichText::new("● 可绘制帧；↳ 保持末帧；— 静态图层。保存 .pcraft 保留动画。").weak())
                    .on_hover_text("点击帧格后在画布绘制。洋葱皮只在暂停时预览所选逐帧图层，不会导出。");
                ui.horizontal(|ui| {
                    let play_icon = if playing { "pause" } else { "play" };
                    if crate::icons::button(ui, play_icon, 22.0, playing, if playing { tl!("Pause") } else { tl!("Play") }).clicked() {
                        toggle_play = true;
                    }
                    if crate::icons::button(ui, "square", 22.0, false, tl!("Stop")).clicked() {
                        if !playing {
                            act = Some(("timeline.setFrame", json!({ "frame": 0 })));
                        }
                        stop_play = true;
                    }
                    if crate::icons::button(ui, "chevron-left", 22.0, false, tl!("Previous frame")).clicked() {
                        stop_play = true;
                        act = Some(("timeline.previousFrame", json!({})));
                    }
                    ui.label(
                        RichText::new(crate::i18n::fmt(
                            crate::i18n::t("Frame {current} / {total}"),
                            &[("current", &(tl.current + 1).to_string()), ("total", &tl.duration.to_string())],
                        ))
                        .color(t.text)
                        .monospace(),
                    );
                    if crate::icons::button(ui, "chevron-right", 22.0, false, tl!("Next frame")).clicked() {
                        stop_play = true;
                        act = Some(("timeline.nextFrame", json!({})));
                    }
                    ui.separator();
                    ui.label(RichText::new(format!("{:.2}s", tl.time())).color(t.text_dim).monospace());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::icons::button(ui, "trash", 20.0, false, tl!("Delete Timeline")).clicked() {
                            stop_play = true;
                            act = Some(("timeline.delete", json!({})));
                        }
                        ui.label(RichText::new(format!("{:.0} fps", tl.fps)).color(t.text_dim));
                    });
                });
                ui.add_space(6.0);
                // Scrubber: frame playhead across the duration.
                if tl.duration > 1 {
                    let mut frame = tl.current as f64;
                    let resp = ui.add(egui::Slider::new(&mut frame, 0.0..=(tl.duration - 1) as f64).integer().show_value(false).text("playhead"));
                    if resp.changed() {
                        stop_play = true;
                        act = Some(("timeline.setFrame", json!({ "frame": frame as u64 })));
                    }
                }
            }
        }
    };
    if let Some(ui) = inline {
        contents(ui);
    } else {
        crate::analysis_ui::panel_window(app, ctx, "timeline", tl!("Timeline"), vec2(0.0, 520.0), 660.0, contents);
    }

    app.ui.timeline.onion_skin = onion;
    app.ui.timeline.reverse_layers = reverse_layers;
    if let Some(id) = select_layer {
        let _ = app.run("layer.select", json!({"layer":id}));
    }

    if close {
        if crate::panel_docking::managed(app, "timeline") {
            let _ = app.run("window.panel.close", json!({"panel":"timeline"}));
        }
        app.ui.timeline.open = false;
        app.ui.timeline.playing = false;
    }
    if stop_play {
        app.ui.timeline.playing = false;
    } else if toggle_play {
        app.ui.timeline.playing = !app.ui.timeline.playing;
        app.ui.timeline.last_time = ctx.input(|i| i.time);
    }
    if let Some((cmd, p)) = act
        && app.run(cmd, p).is_ok()
        && matches!(cmd, "layer.videoLayers.insertBlankFrame" | "layer.videoLayers.duplicateFrame")
    {
        let _ = app.run("timeline.nextFrame", json!({}));
    }
    // Playback: auto-advance the playhead at the frame rate (loops).
    if app.ui.timeline.playing {
        if let Some(t) = app.session.active().and_then(|d| d.doc.timeline.clone()) {
            let now = ctx.input(|i| i.time);
            let period = 1.0 / f64::from(t.fps.max(1.0));
            if now - app.ui.timeline.last_time >= period {
                app.ui.timeline.last_time = now;
                let next = (t.current + 1) % t.duration.max(1);
                let _ = app.run("timeline.setFrame", json!({ "frame": next as u64 }));
            }
            ctx.request_repaint();
        } else {
            app.ui.timeline.playing = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn existing_layers_convert_via_menu_and_docked_tracks_use_available_width() {
        for width in [700.0, 1700.0] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width":32,"height":32})).unwrap();
            app.session
                .edit("fixture", |doc, _| {
                    for i in 0..13 {
                        doc.insert_above(None, photocraft_doc::Layer::raster(format!("Drawing {i}"), doc.pixel_format()));
                    }
                    Ok(())
                })
                .unwrap();
            app.ui.timeline.open = true;
            let mut ready = false;
            let mut h = Harness::builder().with_size(vec2(width, 320.0)).build_ui_state(
                move |ui, app| {
                    if !ready {
                        PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                        ready = true;
                        return;
                    }
                    dock_body(app, ui);
                },
                app,
            );
            h.run_steps(2);
            h.get_by_label("从图层创建帧").click();
            h.run_steps(2);
            h.get_by_label("全部顶层图层 → 帧动画").click();
            h.run_steps(3);
            let st = h.state().session.active().unwrap();
            assert_eq!(st.doc.timeline.as_ref().unwrap().duration, 13);
            assert_eq!(st.doc.layers.iter().filter(|l| l.video.is_some()).count(), 1);
            let last = h.get_by_label("13").rect();
            assert!(last.right() > width - 50.0, "frame columns should fill pane: {last:?}, width={width}");
            let stop = h.get_by_label("Stop").rect();
            assert!(stop.bottom() < 320.0, "playback controls must remain visible: {stop:?}");
            assert!(stop.top() > last.bottom());
        }
    }

    #[test]
    fn timeline_panel_creates_and_edits_real_video_frames() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("timeline.create", json!({"duration": 3, "fps": 12})).unwrap();
        app.ui.timeline.open = true;
        app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1100.0, 900.0));
        let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(|ui, app| windows(app, ui.ctx()), app);
        h.run_steps(2);
        h.get_by_label("新建逐帧图层").click();
        h.run_steps(2);
        let st = h.state().session.active().unwrap();
        let id = st.active_layer.unwrap();
        assert_eq!(st.doc.layer(id).unwrap().video.as_ref().unwrap().frames.len(), 3);
        h.get_by_label("复制当前帧").click();
        h.run_steps(2);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.timeline.as_ref().unwrap().current, 1);
        assert_eq!(st.doc.layer(id).unwrap().video.as_ref().unwrap().frames.len(), 4);
        h.get_by_label("删除当前帧").click();
        h.run_steps(2);
        assert_eq!(h.state().session.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().frames.len(), 3);
        h.get_by_label("洋葱皮（前红 / 后绿）").click();
        h.run_steps(2);
        assert!(h.state().ui.timeline.onion_skin);
    }

    #[test]
    fn stop_step_and_scrub_pause_playback_and_second_stop_rewinds() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app.run("timeline.create", json!({"duration": 10, "fps": 30.0})).unwrap();
        app.ui.timeline.open = true;
        app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1000.0, 700.0));

        let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(0.001).build_ui_state(|ui, app| windows(app, ui.ctx()), app);
        h.run_steps(2);

        // Start playback via Play button.
        h.get_by_label("Play").click();
        h.run_steps(2);
        assert!(h.state().ui.timeline.playing);

        // Move to frame 5 while playing; first Stop pauses at the current frame without rewinding.
        h.state_mut().run("timeline.setFrame", json!({"frame": 5})).unwrap();
        h.get_by_label("Stop").click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing, "first Stop pauses playback");
        assert_eq!(h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current, 5);

        // Second Stop while already paused rewinds to frame 0.
        h.get_by_label("Stop").click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing);
        assert_eq!(h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current, 0);

        // Next frame and Previous frame pause playback.
        h.state_mut().ui.timeline.playing = true;
        h.run_steps(1);
        h.get_by_label("Next frame").click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing, "Next frame pauses playback");
        assert_eq!(h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current, 1);

        h.state_mut().ui.timeline.playing = true;
        h.run_steps(1);
        h.get_by_label("Previous frame").click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing, "Previous frame pauses playback");
        assert_eq!(h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current, 0);

        // Scrubbing the playhead slider pauses playback and updates the current frame.
        h.state_mut().ui.timeline.playing = true;
        h.run_steps(1);
        let slider = h.get_by_role(egui::accesskit::Role::Slider).rect();
        let target = egui::pos2(slider.right() - 8.0, slider.center().y);
        h.event(egui::Event::PointerMoved(target));
        h.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing, "scrubbing pauses playback");
        assert!(h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current > 0);
    }
}
