//! Drawing the SVG of a chart (DESIGN.md, Reports, "Chart functions"): resvg
//! renders it at the size it has on the screen, and the texture is kept, so
//! a chart is rendered again only when its SVG or its size changes.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use resvg::{tiny_skia, usvg};

/// The largest side of a rendered chart, in pixels.
const MAX_SIDE: u32 = 4096;
/// How many rendered charts are kept.
const KEPT: usize = 64;

/// A rendered chart by the hash of its SVG and its size in pixels.
type Kept = HashMap<(u64, u32, u32), egui::TextureHandle>;

#[derive(Default, Clone)]
struct Textures(Arc<Mutex<Kept>>);

/// Paints `svg` into `rect`, scaled to fit and centred, keeping its
/// proportions. SVG that does not parse paints nothing: the layout has
/// already shown why.
pub fn paint_svg(ui: &egui::Ui, painter: &egui::Painter, rect: egui::Rect, svg: &str) {
    if !ui.is_rect_visible(rect) || rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let w = ((rect.width() * ppp).round() as u32).clamp(1, MAX_SIDE);
    let h = ((rect.height() * ppp).round() as u32).clamp(1, MAX_SIDE);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    svg.hash(&mut hasher);
    let key = (hasher.finish(), w, h);
    let textures = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<Textures>(egui::Id::unique("chart_textures"))
            .clone()
    });
    let mut kept = textures.0.lock().unwrap_or_else(|e| e.into_inner());
    let texture = match kept.get(&key) {
        Some(t) => t.clone(),
        None => {
            let Some(image) = render(svg, w, h) else {
                return;
            };
            if kept.len() >= KEPT {
                kept.clear();
            }
            let t = ui.ctx().load_texture(
                format!("chart-{:x}-{w}x{h}", key.0),
                image,
                egui::TextureOptions::LINEAR,
            );
            kept.insert(key, t.clone());
            t
        }
    };
    drop(kept);
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(texture.id(), rect, uv, egui::Color32::WHITE);
}

/// The SVG rendered into a `w` by `h` image, fitted and centred.
pub fn render(svg: &str, w: u32, h: u32) -> Option<egui::ColorImage> {
    let tree = usvg::Tree::from_str(svg, &cliotree_layout::chart::svg_options()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    let size = tree.size();
    let scale = (w as f32 / size.width()).min(h as f32 / size.height());
    let dx = (w as f32 - size.width() * scale) / 2.0;
    let dy = (h as f32 - size.height() * scale) / 2.0;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(egui::ColorImage::from_rgba_premultiplied(
        [w as usize, h as usize],
        pixmap.data(),
    ))
}
