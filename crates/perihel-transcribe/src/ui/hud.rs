//! HUD overlay — 320×64 frameless panel at bottom-center. Never activates
//! (`with_active(false)` → WS_EX_NOACTIVATE), never takes focus, always on top.

use crate::controller::HudState;
use crate::injector::Phase;
use anyhow::Result;
use eframe::egui;
use std::sync::{Arc, Mutex};

const W: f32 = 320.0;
const H: f32 = 64.0;

const ACCENT: egui::Color32 = egui::Color32::from_rgb(0, 140, 200);
const REC_RED: egui::Color32 = egui::Color32::from_rgb(229, 62, 62);
const TEXT: egui::Color32 = egui::Color32::from_rgb(235, 238, 245);

pub fn run(hud: Arc<Mutex<HudState>>) -> Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_always_on_top()
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_active(false) // WS_EX_NOACTIVATE — never steal focus
        .with_inner_size([W, H]);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "perihel-hud",
        options,
        Box::new(move |_cc| {
            position_bottom_center();
            Ok(Box::new(HudApp { hud }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("HUD exited: {e}"))
}

#[cfg(windows)]
fn position_bottom_center() {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    unsafe {
        let sw = GetSystemMetrics(SM_CXSCREEN) as f32;
        let sh = GetSystemMetrics(SM_CYSCREEN) as f32;
        egui::Context::default().send_viewport_cmd(egui::ViewportCommand::OuterPosition(
            egui::pos2((sw - W) / 2.0, sh - H - 90.0),
        ));
    }
}

#[cfg(not(windows))]
fn position_bottom_center() {}

struct HudApp {
    hud: Arc<Mutex<HudState>>,
}

impl eframe::App for HudApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0] // transparent
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // tray menu events
        crate::ui::tray::poll_events();

        let (phase, elapsed, rms, error) = {
            match self.hud.lock() {
                Ok(h) => {
                    let elapsed = h.started_at.map(|t| t.elapsed().unwrap_or_default().as_secs_f32());
                    let rms = h.rms.lock().map(|r| *r).unwrap_or(0.0);
                    (h.phase, elapsed, rms, h.last_error.clone())
                }
                Err(_) => (Phase::Idle, None, 0.0, None),
            }
        };

        let visible = phase != Phase::Idle || error.is_some();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgba_premultiplied(16, 18, 26, 235)).corner_radius(14.0).inner_margin(12.0))
            .show(ctx, |ui| {
                if !visible {
                    return;
                }
                match phase {
                    Phase::Listening => {
                        ui.horizontal(|ui| {
                            pulsing_dot(ui, REC_RED);
                            ui.label(egui::RichText::new(format!("REC {:02.1}s", elapsed.unwrap_or(0.0))).color(TEXT).size(15.0));
                        });
                        rms_bar(ui, rms, REC_RED);
                    }
                    Phase::Transcribing => {
                        ui.horizontal(|ui| {
                            pulsing_dot(ui, ACCENT);
                            ui.label(egui::RichText::new("Transcribing…").color(TEXT).size(15.0));
                        });
                    }
                    Phase::Idle => {
                        if let Some(err) = error {
                            ui.label(egui::RichText::new(err).color(REC_RED).size(12.0));
                        }
                    }
                }
            });

        // only repaint while visible — zero CPU when idle
        if visible {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

fn pulsing_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let t = ui.input(|i| i.time) as f32;
    let alpha = 0.5 + 0.5 * (t * 4.0).sin().abs();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 6.0, color.gamma_multiply(alpha));
}

fn rms_bar(ui: &mut egui::Ui, rms: f32, color: egui::Color32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 4.0), egui::Sense::hover());
    let level = (rms * 6.0).clamp(0.0, 1.0);
    ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(60));
    if level > 0.01 {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(width * level, 4.0)),
            2.0,
            color,
        );
    }
}
