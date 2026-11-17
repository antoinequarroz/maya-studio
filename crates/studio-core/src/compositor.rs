//! Compositeur logiciel (CPU) de référence.
//!
//! Il produit l'image programme en RGBA. C'est la version simple et
//! testable ; le compositeur GPU (wgpu) le remplacera avec la même interface.

use std::collections::HashMap;

use image::RgbaImage;

use crate::model::{Canvas, Scene, Source, SourceKind};

#[derive(Default)]
pub struct Compositor {
    images: HashMap<String, Option<RgbaImage>>,
}

impl Compositor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Dessine la scène dans un buffer RGBA de `canvas.width * canvas.height * 4` octets.
    pub fn render(&mut self, canvas: Canvas, scene: &Scene, frame: u64, out: &mut Vec<u8>) {
        let (w, h) = (canvas.width as usize, canvas.height as usize);
        out.clear();
        out.resize(w * h * 4, 0);
        // Fond noir opaque.
        for px in out.chunks_exact_mut(4) {
            px[3] = 255;
        }
        for source in scene
            .sources
            .iter()
            .filter(|s| s.visible && s.transform.opacity > 0.0)
        {
            self.draw_source(canvas, source, frame, out);
        }
    }

    pub fn render_image(&mut self, canvas: Canvas, scene: &Scene, frame: u64) -> RgbaImage {
        let mut buf = Vec::new();
        self.render(canvas, scene, frame, &mut buf);
        RgbaImage::from_raw(canvas.width, canvas.height, buf).expect("taille cohérente")
    }

    fn draw_source(&mut self, canvas: Canvas, source: &Source, frame: u64, out: &mut [u8]) {
        let t = source.transform;
        let (cw, ch) = (canvas.width as i64, canvas.height as i64);
        let x0 = (t.x.round() as i64).clamp(0, cw);
        let y0 = (t.y.round() as i64).clamp(0, ch);
        let x1 = ((t.x + t.width).round() as i64).clamp(0, cw);
        let y1 = ((t.y + t.height).round() as i64).clamp(0, ch);
        if x1 <= x0 || y1 <= y0 || t.width <= 0.0 || t.height <= 0.0 {
            return;
        }
        let image = match &source.kind {
            SourceKind::Image { path } => {
                let entry = self
                    .images
                    .entry(path.clone())
                    .or_insert_with(|| image::open(path).ok().map(|i| i.to_rgba8()));
                match entry {
                    Some(img) => Some(&*img),
                    None => return, // Fichier illisible : on ignore la source.
                }
            }
            _ => None,
        };
        for y in y0..y1 {
            // Coordonnées normalisées dans la source (0..1).
            let v = ((y as f32 + 0.5) - t.y) / t.height;
            for x in x0..x1 {
                let u = ((x as f32 + 0.5) - t.x) / t.width;
                let px = match &source.kind {
                    SourceKind::Color { rgba } => *rgba,
                    SourceKind::TestPattern => test_pattern(u, v, frame),
                    SourceKind::Image { .. } => {
                        let img = image.expect("image chargée");
                        let sx = ((u * img.width() as f32) as u32).min(img.width() - 1);
                        let sy = ((v * img.height() as f32) as u32).min(img.height() - 1);
                        img.get_pixel(sx, sy).0
                    }
                };
                let i = ((y * cw + x) * 4) as usize;
                blend(&mut out[i..i + 4], px, t.opacity);
            }
        }
    }
}

/// Mélange « source over » d'un pixel RGBA sur le fond.
fn blend(dst: &mut [u8], src: [u8; 4], opacity: f32) {
    let a = (src[3] as f32 / 255.0) * opacity.clamp(0.0, 1.0);
    for c in 0..3 {
        dst[c] = (src[c] as f32 * a + dst[c] as f32 * (1.0 - a)).round() as u8;
    }
    dst[3] = 255;
}

/// Barres de couleur avec une bande blanche qui défile, pour voir que l'image vit.
fn test_pattern(u: f32, v: f32, frame: u64) -> [u8; 4] {
    const BARS: [[u8; 4]; 7] = [
        [192, 192, 192, 255],
        [192, 192, 0, 255],
        [0, 192, 192, 255],
        [0, 192, 0, 255],
        [192, 0, 192, 255],
        [192, 0, 0, 255],
        [0, 0, 192, 255],
    ];
    let sweep = (frame % 120) as f32 / 120.0;
    if (u - sweep).abs() < 0.01 {
        return [255, 255, 255, 255];
    }
    if v > 0.8 {
        let g = (u * 255.0) as u8;
        return [g, g, g, 255];
    }
    BARS[((u * 7.0) as usize).min(6)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Scene, Transform};
    use uuid::Uuid;

    #[test]
    fn color_source_fills_its_rect() {
        let canvas = Canvas {
            width: 4,
            height: 2,
            fps: 30,
        };
        let mut scene = Scene::new("t");
        scene.sources.push(Source {
            id: Uuid::new_v4(),
            name: "rouge".into(),
            kind: SourceKind::Color {
                rgba: [255, 0, 0, 255],
            },
            transform: Transform {
                x: 2.0,
                y: 0.0,
                width: 2.0,
                height: 2.0,
                opacity: 1.0,
            },
            visible: true,
        });
        let img = Compositor::new().render_image(canvas, &scene, 0);
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(img.get_pixel(3, 1).0, [255, 0, 0, 255]);
    }

    #[test]
    fn opacity_blends() {
        let mut px = [0, 0, 0, 255];
        blend(&mut px, [200, 100, 0, 255], 0.5);
        assert_eq!(px, [100, 50, 0, 255]);
    }
}
