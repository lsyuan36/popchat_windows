//! Test-only, opt-in software snapshots of the real egui output. No window,
//! network, clipboard, user configuration or credentials are accessed.
use eframe::egui;
use std::{collections::HashMap, path::Path};

/// Egui groups a frame's shadow and fill into Shape::Vec. Geometry assertions
/// must inspect those children, not just top-level clipped shapes.
pub(crate) fn rect_shapes(output: &egui::FullOutput) -> Vec<&egui::epaint::RectShape> {
    fn collect<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::epaint::RectShape>) {
        match shape {
            egui::Shape::Rect(rect) => out.push(rect),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = vec![];
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut out);
    }
    out
}

pub(crate) fn prepare_fonts(ctx: &egui::Context) {
    if std::env::var_os("POPCHAT_MCP_UI_PREVIEW_DIR").is_none() {
        return;
    }
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "preview-cjk".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/swei-gothic/SweiGothicCJKtc-Regular.ttf"
        ))),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .get_mut(&family)
            .unwrap()
            .push("preview-cjk".into());
    }
    ctx.set_fonts(fonts);
}

pub(crate) fn export(
    ctx: &egui::Context,
    frames: &[egui::FullOutput],
    size: egui::Vec2,
    name: &str,
    dark: bool,
) {
    let Some(directory) = std::env::var_os("POPCHAT_MCP_UI_PREVIEW_DIR") else {
        return;
    };
    let directory = Path::new(&directory);
    std::fs::create_dir_all(directory).unwrap();
    let background = if dark {
        [44, 48, 60, 255]
    } else {
        [230, 224, 212, 255]
    };
    render(ctx, frames, size, background)
        .save(directory.join(name))
        .unwrap();
}

pub(crate) fn render(
    ctx: &egui::Context,
    frames: &[egui::FullOutput],
    size: egui::Vec2,
    background: [u8; 4],
) -> image::RgbaImage {
    let mut textures = HashMap::<egui::TextureId, egui::ColorImage>::new();
    for frame in frames {
        for (id, delta) in &frame.textures_delta.set {
            let egui::ImageData::Color(image) = &delta.image;
            if let Some([x, y]) = delta.pos {
                let target = textures
                    .get_mut(id)
                    .expect("texture patch requires an atlas");
                for row in 0..image.size[1] {
                    let start = (y + row) * target.size[0] + x;
                    target.pixels[start..start + image.size[0]].copy_from_slice(
                        &image.pixels[row * image.size[0]..(row + 1) * image.size[0]],
                    );
                }
            } else {
                textures.insert(*id, image.as_ref().clone());
            }
        }
    }
    let last = frames.last().unwrap();
    let scale = last.pixels_per_point;
    let width = (size.x * scale).round() as u32;
    let height = (size.y * scale).round() as u32;
    let mut canvas = image::RgbaImage::from_pixel(width, height, image::Rgba(background));
    for primitive in ctx.tessellate(last.shapes.clone(), scale) {
        let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
            panic!("unexpected native callback in headless UI");
        };
        let texture = textures
            .get(&mesh.texture_id)
            .expect("missing snapshot texture");
        let clip = primitive.clip_rect * scale;
        for triangle in mesh.indices.chunks_exact(3) {
            let v = [
                mesh.vertices[triangle[0] as usize],
                mesh.vertices[triangle[1] as usize],
                mesh.vertices[triangle[2] as usize],
            ];
            let positions = v.map(|v| v.pos * scale);
            let cross = |a: egui::Vec2, b: egui::Vec2| a.x * b.y - a.y * b.x;
            let area = cross(positions[1] - positions[0], positions[2] - positions[0]);
            if area.abs() < 0.0001 {
                continue;
            }
            let bounds = egui::Rect::from_points(&positions)
                .intersect(clip)
                .intersect(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width as f32, height as f32),
                ));
            for y in bounds.top().floor().max(0.0) as u32
                ..bounds.bottom().ceil().min(height as f32) as u32
            {
                for x in bounds.left().floor().max(0.0) as u32
                    ..bounds.right().ceil().min(width as f32) as u32
                {
                    let point = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                    let weights = [
                        cross(positions[1] - point, positions[2] - point) / area,
                        cross(positions[2] - point, positions[0] - point) / area,
                        cross(positions[0] - point, positions[1] - point) / area,
                    ];
                    if weights.iter().any(|v| *v < 0.0) {
                        continue;
                    }
                    // Shared edges belong to one triangle, not both. This
                    // avoids false diagonals when alpha-blending previews.
                    if weights.iter().enumerate().any(|(index, weight)| {
                        if weight.abs() > 0.000001 {
                            return false;
                        }
                        let edge = (positions[(index + 2) % 3] - positions[(index + 1) % 3])
                            * area.signum();
                        !(edge.y < 0.0 || (edge.y == 0.0 && edge.x > 0.0))
                    }) {
                        continue;
                    }
                    let uv = v
                        .iter()
                        .zip(weights)
                        .fold(egui::Vec2::ZERO, |sum, (v, w)| sum + v.uv.to_vec2() * w);
                    let tx = (uv.x * texture.size[0] as f32 - 0.5)
                        .clamp(0.0, (texture.size[0] - 1) as f32);
                    let ty = (uv.y * texture.size[1] as f32 - 0.5)
                        .clamp(0.0, (texture.size[1] - 1) as f32);
                    let at =
                        |x: usize, y: usize| texture.pixels[y * texture.size[0] + x].to_array();
                    let x0 = tx.floor() as usize;
                    let y0 = ty.floor() as usize;
                    let x1 = (x0 + 1).min(texture.size[0] - 1);
                    let y1 = (y0 + 1).min(texture.size[1] - 1);
                    let colors = [at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1)];
                    let sx = tx - x0 as f32;
                    let sy = ty - y0 as f32;
                    let tex_weights = [
                        (1.0 - sx) * (1.0 - sy),
                        sx * (1.0 - sy),
                        (1.0 - sx) * sy,
                        sx * sy,
                    ];
                    let src: [f32; 4] = std::array::from_fn(|channel| {
                        let tint: f32 = v
                            .iter()
                            .zip(weights)
                            .map(|(v, w)| v.color.to_array()[channel] as f32 * w)
                            .sum();
                        let texel: f32 = colors
                            .iter()
                            .zip(tex_weights)
                            .map(|(c, w)| c[channel] as f32 * w)
                            .sum();
                        tint * texel / 255.0
                    });
                    let dst = canvas.get_pixel_mut(x, y);
                    let inverse = 1.0 - src[3] / 255.0;
                    for channel in 0..3 {
                        dst.0[channel] = (src[channel] + dst.0[channel] as f32 * inverse)
                            .round()
                            .clamp(0.0, 255.0) as u8;
                    }
                    dst.0[3] = (src[3] + dst.0[3] as f32 * inverse)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    // Epaint colors above are premultiplied; PNG and image::Rgba use straight
    // alpha. Opaque legacy previews are unchanged by this conversion.
    for pixel in canvas.pixels_mut() {
        if pixel.0[3] != 0 && pixel.0[3] != 255 {
            for channel in 0..3 {
                pixel.0[channel] = (f32::from(pixel.0[channel]) * 255.0 / f32::from(pixel.0[3]))
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
    canvas
}
