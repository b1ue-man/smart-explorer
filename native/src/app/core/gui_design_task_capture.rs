//! Record real egui geometry and texture data for remote software rendering.
//! This is a test artifact writer, not a second implementation of the GUI.
use base64::Engine;
use eframe::egui::{self, epaint, Color32, ColorImage};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

#[derive(Default)]
pub(super) struct Capture {
    textures: HashMap<egui::TextureId, ColorImage>,
    pub(super) labels: Vec<(String, egui::Rect, egui::Rect)>,
}

impl Capture {
    pub(super) fn record(&mut self, output: &egui::FullOutput) {
        self.labels.clear();
        for clipped in &output.shapes {
            self.record_labels(&clipped.shape, clipped.clip_rect);
        }
        for (id, delta) in &output.textures_delta.set {
            let image = match &delta.image {
                epaint::ImageData::Color(image) => (**image).clone(),
                epaint::ImageData::Font(image) => ColorImage {
                    size: image.size, pixels: image.srgba_pixels(None).collect(),
                },
            };
            if let Some([x, y]) = delta.pos {
                let target = self.textures.get_mut(id).expect("texture before partial update");
                for row in 0..image.size[1] {
                    let start = (y + row) * target.size[0] + x;
                    target.pixels[start..start + image.size[0]]
                        .copy_from_slice(&image.pixels[row * image.size[0]..(row + 1) * image.size[0]]);
                }
            } else {
                self.textures.insert(*id, image);
            }
        }
    }

    fn record_labels(&mut self, shape: &epaint::Shape, clip: egui::Rect) {
        match shape {
            epaint::Shape::Text(text) => self.labels.push((text.galley.job.text.clone(),
                egui::Rect::from_min_size(text.pos, text.galley.size()), clip)),
            epaint::Shape::Vec(shapes) => {
                for shape in shapes { self.record_labels(shape, clip); }
            }
            _ => {}
        }
    }

    pub(super) fn contains(&self, label: &str) -> bool {
        self.labels.iter().any(|(text, rect, clip)| text == label && rect.intersects(*clip))
    }

    pub(super) fn target(&self, label: &str) -> egui::Pos2 {
        self.labels.iter().rev().find(|(text, rect, clip)| text == label && clip.contains(rect.center()))
            .unwrap_or_else(|| panic!("missing visible target {label:?}; labels: {:?}", self.labels))
            .1.center()
    }

    pub(super) fn save(&self, ctx: &egui::Context, output: egui::FullOutput, path: &Path,
        size: egui::Vec2) {
        let meshes: Vec<_> = ctx.tessellate(output.shapes, output.pixels_per_point).into_iter().map(|clipped| {
            let epaint::Primitive::Mesh(mesh) = clipped.primitive else { panic!("unexpected paint callback") };
            json!({
                "clip": [clipped.clip_rect.min.x, clipped.clip_rect.min.y, clipped.clip_rect.max.x, clipped.clip_rect.max.y],
                "texture": format!("{:?}", mesh.texture_id), "indices": mesh.indices,
                "vertices": mesh.vertices.iter().map(|vertex| [vertex.pos.x, vertex.pos.y,
                    vertex.uv.x, vertex.uv.y, f32::from(vertex.color.r()), f32::from(vertex.color.g()),
                    f32::from(vertex.color.b()), f32::from(vertex.color.a())]).collect::<Vec<_>>()
            })
        }).collect();
        let textures: Vec<_> = self.textures.iter().map(|(id, image)| {
            let bytes: Vec<_> = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
            json!({"id": format!("{id:?}"), "size": image.size,
                "rgba": base64::engine::general_purpose::STANDARD.encode(bytes)})
        }).collect();
        let value = json!({"width": size.x, "height": size.y, "pixels_per_point": output.pixels_per_point,
            "meshes": meshes, "textures": textures});
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
}
