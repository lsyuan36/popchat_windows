//! All-Rust custom accent swatch/popover, ported from `AccentPicker.swift`.
//! Slider state is intentionally independent of stored, quantized sRGB: reading
//! our own writes back would lose hue on gray and saturation at black.

use eframe::egui;

use super::theme::{hex_from_hsb, hex_to_color32, hsb_from_hex, DEFAULT_ACCENT_HEX};

pub struct AccentPicker {
    open: bool,
    hue: f64,
    saturation: f64,
    brightness: f64,
    hex_draft: String,
    observed_custom: String,
    #[cfg(test)]
    geometry: TestGeometry,
}

impl Default for AccentPicker {
    fn default() -> Self {
        Self {
            open: false,
            hue: 0.0,
            saturation: 1.0,
            brightness: 1.0,
            hex_draft: String::new(),
            observed_custom: String::new(),
            #[cfg(test)]
            geometry: TestGeometry::default(),
        }
    }
}

impl AccentPicker {
    /// Draw the rainbow-ring custom swatch and its live HSB/hex popover.
    /// Existing custom colors are selected when the swatch is clicked. Merely
    /// opening an unset swatch never publishes an accent or creates a custom.
    pub fn show(&mut self, ui: &mut egui::Ui, accent_hex: &mut String, custom_hex: &mut String) {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Custom accent")
        });
        paint_swatch(
            ui,
            rect,
            custom_hex,
            !custom_hex.is_empty() && accent_hex == custom_hex,
        );
        #[cfg(test)]
        {
            self.geometry.swatch = Some(rect);
        }
        if response.clicked() {
            if !custom_hex.is_empty() && accent_hex != custom_hex {
                accent_hex.clone_from(custom_hex);
            }
            if !self.open {
                self.seed(if custom_hex.is_empty() {
                    accent_hex
                } else {
                    custom_hex
                });
                self.observed_custom.clone_from(custom_hex);
            }
            self.open = true;
        }
        let response = response.on_hover_text(if custom_hex.is_empty() {
            "Choose a custom accent color".to_owned()
        } else {
            format!("Custom accent {custom_hex} — click to edit")
        });

        // Use a local open flag so the popup can borrow the rest of this state.
        // The native egui popup handles outside clicks/Escape, not slider clicks.
        let mut open = self.open;
        egui::Popup::from_response(&response)
            .open_bool(&mut open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .gap(4.0)
            .width(268.0)
            .frame(egui::Frame::popup(ui.style()).inner_margin(14))
            .show(|ui| {
                self.sync_custom(accent_hex, custom_hex);
                self.popover_body(ui, accent_hex, custom_hex);
            });
        self.open = open;
    }

    /// Seed writes only local state. Gray has no hue, so retain the previous one.
    fn seed(&mut self, hex: &str) {
        let (h, s, b) = hsb_from_hex(hex);
        if s > 0.0 {
            self.hue = h;
        }
        self.saturation = s;
        self.brightness = b;
        self.hex_draft = hex_from_hsb(self.hue, s, b);
    }

    /// Equivalent to the source's customHex onChange, not an accent onChange.
    fn sync_custom(&mut self, accent_hex: &str, custom_hex: &str) {
        if self.observed_custom != custom_hex {
            self.observed_custom = custom_hex.to_owned();
            if custom_hex != self.hex_draft {
                self.seed(if custom_hex.is_empty() {
                    accent_hex
                } else {
                    custom_hex
                });
            }
        }
    }

    /// Return whether a stored value was actually published. No self-reseeding:
    /// at black, saturation must survive even though black's parsed HSB has s=0.
    fn apply(&mut self, accent_hex: &mut String, custom_hex: &mut String) -> bool {
        let hex = hex_from_hsb(self.hue, self.saturation, self.brightness);
        if hex == *custom_hex && hex == *accent_hex {
            return false;
        }
        self.hex_draft.clone_from(&hex);
        custom_hex.clone_from(&hex);
        *accent_hex = hex;
        self.observed_custom.clone_from(custom_hex);
        true
    }

    fn commit_hex_draft(&mut self, accent_hex: &mut String, custom_hex: &mut String) -> bool {
        // Swift uses .whitespaces, not .whitespacesAndNewlines.
        let trimmed = self.hex_draft.trim_matches(|c: char| {
            c.is_whitespace() && !matches!(c, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
        });
        let body = trimmed.strip_prefix('#').unwrap_or(trimmed);
        if body.len() != 6 || !body.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            self.hex_draft.clone_from(if custom_hex.is_empty() {
                accent_hex
            } else {
                custom_hex
            });
            return false;
        }
        let normalized = format!("#{}", body.to_ascii_uppercase());
        self.seed(&normalized);
        self.apply(accent_hex, custom_hex)
    }

    fn reset(&mut self, accent_hex: &mut String, custom_hex: &mut String) {
        custom_hex.clear();
        if accent_hex != DEFAULT_ACCENT_HEX {
            *accent_hex = DEFAULT_ACCENT_HEX.to_owned();
        }
        self.sync_custom(accent_hex, custom_hex);
    }

    fn popover_body(
        &mut self,
        ui: &mut egui::Ui,
        accent_hex: &mut String,
        custom_hex: &mut String,
    ) {
        ui.set_width(240.0);
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 7.0, hex_to_color32(&self.hex_draft));
            ui.painter().rect_stroke(
                rect,
                7.0,
                egui::Stroke::new(0.5_f32, ui.visuals().text_color().gamma_multiply(0.12)),
                egui::StrokeKind::Inside,
            );
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("Custom accent").size(12.0).strong());
                ui.label(
                    egui::RichText::new("Used everywhere the accent appears")
                        .size(10.5)
                        .weak(),
                );
            });
        });
        ui.add_space(4.0);

        for (index, label) in ["Hue", "Saturation", "Brightness"].into_iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [62.0, 15.0],
                    egui::Label::new(egui::RichText::new(label).size(11.0).weak()),
                );
                let colors: Vec<_> = match index {
                    0 => (0..=12)
                        .map(|step| hex_to_color32(&hex_from_hsb(f64::from(step) / 12.0, 1.0, 1.0)))
                        .collect(),
                    1 => [0.0, 1.0]
                        .map(|s| hex_to_color32(&hex_from_hsb(self.hue, s, self.brightness)))
                        .to_vec(),
                    _ => vec![
                        egui::Color32::BLACK,
                        hex_to_color32(&hex_from_hsb(self.hue, self.saturation, 1.0)),
                    ],
                };
                let value = match index {
                    0 => &mut self.hue,
                    1 => &mut self.saturation,
                    _ => &mut self.brightness,
                };
                let response = gradient_slider(ui, label, value, &colors);
                #[cfg(test)]
                {
                    self.geometry.sliders[index] = Some(response.rect);
                }
                if response.changed() {
                    self.apply(accent_hex, custom_hex);
                }
            });
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_sized(
                [62.0, 20.0],
                egui::Label::new(egui::RichText::new("Hex").size(11.0).weak()),
            );
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.hex_draft)
                    .hint_text("#RRGGBB")
                    .font(egui::FontId::monospace(11.5))
                    .desired_width(82.0),
            );
            #[cfg(test)]
            {
                self.geometry.hex = Some(field.rect);
            }
            if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.commit_hex_draft(accent_hex, custom_hex);
            }
            let reset = ui
                .small_button("Reset")
                .on_hover_text("Forget the custom color and go back to Blue");
            #[cfg(test)]
            {
                self.geometry.reset = Some(reset.rect);
            }
            if reset.clicked() {
                self.reset(accent_hex, custom_hex);
            }
        });
    }
}

fn mix_stops(colors: &[egui::Color32], fraction: f32) -> egui::Color32 {
    let position = fraction.clamp(0.0, 1.0) * (colors.len() - 1) as f32;
    let low = (position.floor() as usize).min(colors.len() - 1);
    let high = (low + 1).min(colors.len() - 1);
    let t = position - low as f32;
    let channel = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    egui::Color32::from_rgb(
        channel(colors[low].r(), colors[high].r()),
        channel(colors[low].g(), colors[high].g()),
        channel(colors[low].b(), colors[high].b()),
    )
}

fn paint_swatch(ui: &egui::Ui, rect: egui::Rect, custom_hex: &str, selected: bool) {
    let colors = [
        "#FF3B30", "#FF9F0A", "#30D158", "#0A84FF", "#BF5AF2", "#FF3B30",
    ]
    .map(hex_to_color32);
    let center = rect.center();
    let mut mesh = egui::Mesh::default();
    for step in 0..120 {
        let start = step as f32 / 120.0;
        let end = (step + 1) as f32 / 120.0;
        let point = |t: f32| center + egui::Vec2::angled(t * std::f32::consts::TAU) * 9.0;
        let index = mesh.vertices.len() as u32;
        mesh.colored_vertex(center, mix_stops(&colors, (start + end) * 0.5));
        mesh.colored_vertex(point(start), mix_stops(&colors, start));
        mesh.colored_vertex(point(end), mix_stops(&colors, end));
        mesh.add_triangle(index, index + 1, index + 2);
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    if !custom_hex.is_empty() {
        ui.painter()
            .circle_filled(center, 6.0, hex_to_color32(custom_hex));
    }
    if selected {
        ui.painter().circle_stroke(
            center,
            10.0,
            egui::Stroke::new(2.0_f32, hex_to_color32(custom_hex)),
        );
    }
}

/// A capsule filled with the actual sRGB ramp, plus a white 15-point thumb.
/// Zero-distance pointer drags map through the thumb's inset span, as in Swift.
fn gradient_slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    colors: &[egui::Color32],
) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(16.0), 15.0),
        egui::Sense::click_and_drag(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), label)
    });
    let previous = *value;
    let mut edited = false;
    let span = (rect.width() - 15.0).max(1.0);
    if (response.clicked() || response.dragged() || response.is_pointer_button_down_on())
        && ui.is_enabled()
    {
        if let Some(pointer) = response.interact_pointer_pos() {
            *value = f64::from(((pointer.x - rect.left() - 7.5) / span).clamp(0.0, 1.0));
            edited = true;
            response.request_focus();
        }
    }
    if response.has_focus() && ui.is_enabled() {
        ui.input(|input| {
            if input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::ArrowDown) {
                *value -= 1.0 / 255.0;
                edited = true;
            }
            if input.key_pressed(egui::Key::ArrowRight) || input.key_pressed(egui::Key::ArrowUp) {
                *value += 1.0 / 255.0;
                edited = true;
            }
            if input.key_pressed(egui::Key::Home) {
                *value = 0.0;
                edited = true;
            }
            if input.key_pressed(egui::Key::End) {
                *value = 1.0;
                edited = true;
            }
        });
        *value = value.clamp(0.0, 1.0);
    }
    // A zero-distance gesture still invokes the Swift binding setter. It can
    // select/create a custom even when the HSB value itself didn't move.
    if edited || *value != previous {
        response.mark_changed();
    }

    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), 9.0));
    let mut mesh = egui::Mesh::default();
    for column in 0..=96 {
        let t = column as f32 / 96.0;
        let x = track.left() + track.width() * t;
        let edge = (x - track.left()).min(track.right() - x);
        let half_height = if edge < 4.5 {
            (4.5_f32.powi(2) - (4.5 - edge).powi(2)).max(0.0).sqrt()
        } else {
            4.5
        };
        let color = mix_stops(colors, t);
        mesh.colored_vertex(egui::pos2(x, track.center().y - half_height), color);
        mesh.colored_vertex(egui::pos2(x, track.center().y + half_height), color);
        if column > 0 {
            let index = column * 2;
            mesh.add_triangle(index - 2, index - 1, index);
            mesh.add_triangle(index - 1, index + 1, index);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    ui.painter().rect_stroke(
        track,
        4.5,
        egui::Stroke::new(0.5_f32, ui.visuals().text_color().gamma_multiply(0.10)),
        egui::StrokeKind::Inside,
    );
    let center = egui::pos2(
        rect.left() + 7.5 + value.clamp(0.0, 1.0) as f32 * span,
        rect.center().y,
    );
    ui.painter().circle_filled(
        center + egui::vec2(0.0, 0.5),
        8.0,
        egui::Color32::from_black_alpha(30),
    );
    ui.painter()
        .circle_filled(center, 7.5, egui::Color32::WHITE);
    ui.painter().circle_stroke(
        center,
        7.25,
        egui::Stroke::new(0.5_f32, egui::Color32::from_black_alpha(56)),
    );
    if response.has_focus() {
        ui.painter()
            .circle_stroke(center, 8.5, ui.visuals().selection.stroke);
    }
    response
}

#[cfg(test)]
#[derive(Default)]
struct TestGeometry {
    swatch: Option<egui::Rect>,
    sliders: [Option<egui::Rect>; 3],
    hex: Option<egui::Rect>,
    reset: Option<egui::Rect>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_never_publishes_and_gray_retains_hue() {
        let mut picker = AccentPicker::default();
        let accent = "#30D158".to_owned();
        let custom = String::new();
        picker.seed(&accent);
        let hue = picker.hue;
        picker.seed("#808080");
        assert_eq!(picker.hue, hue);
        assert_eq!(picker.saturation, 0.0);
        assert_eq!(picker.hex_draft, "#808080");
        picker.seed("#000000");
        assert_eq!(picker.hue, hue);
        assert_eq!(picker.saturation, 0.0); // Explicit seed is not a slider write.
        assert_eq!(accent, "#30D158");
        assert!(custom.is_empty());
    }

    #[test]
    fn slider_black_retains_saturation_and_gray_retains_hue_without_echo() {
        let mut picker = AccentPicker::default();
        let mut accent = "#BF5AF2".to_owned();
        let mut custom = accent.clone();
        picker.seed(&custom);
        let (h, s, b) = (picker.hue, picker.saturation, picker.brightness);
        picker.brightness = 0.0;
        assert!(picker.apply(&mut accent, &mut custom));
        picker.sync_custom(&accent, &custom);
        assert_eq!((picker.hue, picker.saturation), (h, s));
        assert_eq!(custom, "#000000");
        picker.hue = 0.2; // Changing H/S at black keeps state, but publishes nothing.
        picker.saturation = 0.6;
        assert!(!picker.apply(&mut accent, &mut custom));
        picker.sync_custom(&accent, &custom);
        assert_eq!((picker.hue, picker.saturation), (0.2, 0.6));
        picker.brightness = b;
        assert!(picker.apply(&mut accent, &mut custom));
        assert_eq!(custom, hex_from_hsb(0.2, 0.6, b));
        picker.saturation = 0.0;
        picker.apply(&mut accent, &mut custom);
        picker.sync_custom(&accent, &custom);
        assert_eq!(picker.hue, 0.2);
        picker.saturation = s;
        picker.apply(&mut accent, &mut custom);
        assert_eq!(custom, hex_from_hsb(0.2, s, b));
    }

    #[test]
    fn same_eight_bit_color_is_deduplicated_but_selecting_custom_still_applies() {
        let mut picker = AccentPicker::default();
        let mut accent = "#FF0000".to_owned();
        let mut custom = accent.clone();
        picker.seed(&custom);
        picker.brightness = 0.99999;
        assert!(!picker.apply(&mut accent, &mut custom));
        assert_eq!(custom, "#FF0000");
        accent = DEFAULT_ACCENT_HEX.to_owned();
        assert!(picker.apply(&mut accent, &mut custom));
        assert_eq!(accent, custom);
        assert!(!picker.apply(&mut accent, &mut custom));
    }

    #[test]
    fn commit_normalizes_valid_hex_and_rejects_bad_drafts_without_guessing() {
        let mut picker = AccentPicker::default();
        let mut accent = "#30D158".to_owned();
        let mut custom = String::new();
        picker.seed(&accent);
        for bad in [
            "",
            "#123",
            "#1234567",
            "#gggggg",
            "##123456",
            "你好",
            "12345é",
            "\n123456\n",
            "+12345",
        ] {
            picker.hex_draft = bad.to_owned();
            assert!(!picker.commit_hex_draft(&mut accent, &mut custom), "{bad}");
            assert_eq!(picker.hex_draft, accent);
            assert!(custom.is_empty());
        }
        picker.hex_draft = " \tbf5af2\t ".to_owned();
        assert!(picker.commit_hex_draft(&mut accent, &mut custom));
        assert_eq!(custom, "#BF5AF2");
        assert_eq!(accent, custom);
        let hue = picker.hue;
        picker.hex_draft = "#808080".to_owned();
        assert!(picker.commit_hex_draft(&mut accent, &mut custom));
        assert_eq!(picker.hue, hue);
        assert_eq!(picker.saturation, 0.0);
        accent = DEFAULT_ACCENT_HEX.to_owned();
        picker.hex_draft = "bad".to_owned();
        assert!(!picker.commit_hex_draft(&mut accent, &mut custom));
        assert_eq!(picker.hex_draft, "#808080"); // Current custom, not active preset.
        assert_eq!(accent, DEFAULT_ACCENT_HEX);
    }

    #[test]
    fn external_custom_changes_reseed_without_publishing_and_reset_stays_unset() {
        let mut picker = AccentPicker::default();
        let mut accent = "#BF5AF2".to_owned();
        let mut custom = accent.clone();
        picker.seed(&custom);
        picker.observed_custom.clone_from(&custom);
        custom = "#808080".to_owned();
        let hue = picker.hue;
        picker.sync_custom(&accent, &custom);
        assert_eq!(picker.hue, hue);
        assert_eq!(picker.hex_draft, custom);
        assert_eq!(accent, "#BF5AF2");
        picker.reset(&mut accent, &mut custom);
        assert!(custom.is_empty());
        assert_eq!(accent, DEFAULT_ACCENT_HEX);
        assert_eq!(picker.hex_draft, DEFAULT_ACCENT_HEX);
        picker.sync_custom(&accent, &custom);
        assert!(custom.is_empty());
        // An accent-only preset change isn't an outside customHex change.
        accent = "#FF9F0A".to_owned();
        picker.sync_custom(&accent, &custom);
        assert_eq!(picker.hex_draft, DEFAULT_ACCENT_HEX);
    }

    fn frame(
        ctx: &egui::Context,
        picker: &mut AccentPicker,
        accent: &mut String,
        custom: &mut String,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
            events,
            ..Default::default()
        };
        ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| picker.show(ui, accent, custom));
        })
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    fn replace_and_submit(text: &str) -> Vec<egui::Event> {
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            },
            egui::Event::Text(text.to_owned()),
            egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn headless_zero_distance_gray_selection_and_actual_hex_submissions() {
        let ctx = egui::Context::default();
        let mut picker = AccentPicker::default();
        let mut accent = "#808080".to_owned();
        let mut custom = String::new();
        let _ = frame(&ctx, &mut picker, &mut accent, &mut custom, vec![]);
        let swatch = picker.geometry.swatch.unwrap().center();
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(swatch, true),
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(swatch, false),
        );
        for _ in 0..2 {
            let _ = frame(&ctx, &mut picker, &mut accent, &mut custom, vec![]);
        }
        assert!(custom.is_empty());
        let hue = picker.geometry.sliders[0].unwrap();
        let zero = egui::pos2(hue.left() + 7.5, hue.center().y);
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(zero, true),
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(zero, false),
        );
        assert_eq!(picker.hue, 0.0);
        assert_eq!(
            custom, "#808080",
            "Even a zero-distance slider gesture is an edit"
        );

        let field = picker.geometry.hex.unwrap().center();
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(field, true),
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(field, false),
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            replace_and_submit("invalid"),
        );
        assert_eq!(picker.hex_draft, "#808080");
        assert_eq!(custom, "#808080");
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(field, true),
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(field, false),
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            replace_and_submit("bf5af2"),
        );
        assert_eq!(picker.hex_draft, "#BF5AF2");
        assert_eq!(accent, "#BF5AF2");
        assert_eq!(custom, "#BF5AF2");
    }

    #[test]
    fn headless_real_swatch_popup_drag_reset_and_dismissal() {
        let ctx = egui::Context::default();
        let mut picker = AccentPicker::default();
        let mut accent = DEFAULT_ACCENT_HEX.to_owned();
        let mut custom = String::new();
        let _ = frame(&ctx, &mut picker, &mut accent, &mut custom, vec![]);
        let swatch = picker.geometry.swatch.unwrap().center();
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(swatch, true),
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(swatch, false),
        );
        assert!(picker.open);
        assert!(custom.is_empty(), "Opening must not publish");
        let _ = frame(&ctx, &mut picker, &mut accent, &mut custom, vec![]);
        let output = frame(&ctx, &mut picker, &mut accent, &mut custom, vec![]);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Custom accent")));
        assert!(
            output
                .shapes
                .iter()
                .filter(|shape| matches!(&shape.shape, egui::Shape::Mesh(_)))
                .count()
                >= 4,
            "Rainbow swatch plus three gradient tracks must actually paint"
        );
        let hue = picker.geometry.sliders[0].unwrap();
        let start = egui::pos2(
            hue.left() + 7.5 + (hue.width() - 15.0) * 0.75,
            hue.center().y,
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(start, true),
        );
        assert!(!custom.is_empty());
        let finish = egui::pos2(
            hue.left() + 7.5 + (hue.width() - 15.0) * 0.25,
            hue.center().y,
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            vec![egui::Event::PointerMoved(finish)],
        );
        assert!((picker.hue - 0.25).abs() < 0.001);
        assert_eq!(
            accent,
            hex_from_hsb(picker.hue, picker.saturation, picker.brightness)
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(finish, false),
        );
        assert!(picker.open, "Editing inside the popup must not dismiss it");
        let reset = picker.geometry.reset.unwrap().center();
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(reset, true),
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(reset, false),
        );
        assert!(custom.is_empty());
        assert_eq!(accent, DEFAULT_ACCENT_HEX);
        let _ = frame(&ctx, &mut picker, &mut accent, &mut custom, vec![]);
        assert!(
            custom.is_empty(),
            "Reset must not seed-publish a new custom color"
        );
        let outside = egui::pos2(600.0, 400.0);
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(outside, true),
        );
        let _ = frame(
            &ctx,
            &mut picker,
            &mut accent,
            &mut custom,
            pointer(outside, false),
        );
        assert!(!picker.open);
    }
}
