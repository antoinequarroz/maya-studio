//! Compositeur logiciel (CPU) de référence.
//!
//! Il produit l'image programme en RGBA. C'est la version simple et
//! testable ; le compositeur GPU (wgpu) le remplacera avec la même interface.

use std::collections::HashMap;
use std::sync::Arc;

use image::RgbaImage;
use uuid::Uuid;

use crate::model::{Canvas, Scene, Source, SourceKind};

/// Image RGBA d'une source de capture (lignes contiguës, `width * 4` octets par ligne).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl VideoFrame {
    /// `None` si la taille des données ne correspond pas aux dimensions.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        (width > 0 && height > 0 && data.len() == width as usize * height as usize * 4).then_some(
            Self {
                width,
                height,
                data,
            },
        )
    }
}

/// Fournit la dernière image des sources de capture (caméra, écran, fichier…).
///
/// Le moteur l'implémente avec les images reçues de GStreamer ; le cœur reste
/// ainsi indépendant de GStreamer et testable avec des images synthétiques.
pub trait FrameProvider {
    /// Dernière image de la source, ou `None` si elle n'en a pas (encore).
    fn frame(&self, source_id: Uuid) -> Option<Arc<VideoFrame>>;
}

/// Fournisseur vide : les sources de capture ne dessinent rien.
pub struct NoFrames;

impl FrameProvider for NoFrames {
    fn frame(&self, _source_id: Uuid) -> Option<Arc<VideoFrame>> {
        None
    }
}

impl<F: Fn(Uuid) -> Option<Arc<VideoFrame>>> FrameProvider for F {
    fn frame(&self, source_id: Uuid) -> Option<Arc<VideoFrame>> {
        self(source_id)
    }
}

#[derive(Default)]
pub struct Compositor {
    images: HashMap<String, Option<RgbaImage>>,
}

/// Pixels à dessiner pour une source.
enum Pixels<'a> {
    Solid([u8; 4]),
    Pattern,
    /// Image source mise à l'échelle (plus proche voisin) dans le rectangle.
    Raster {
        width: u32,
        height: u32,
        data: &'a [u8],
    },
}

impl Compositor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Dessine la scène dans un buffer RGBA de `canvas.width * canvas.height * 4` octets,
    /// sans images de capture (voir [`Compositor::render_with`]).
    pub fn render(&mut self, canvas: Canvas, scene: &Scene, frame: u64, out: &mut Vec<u8>) {
        self.render_with(canvas, scene, frame, &NoFrames, out);
    }

    /// Dessine la scène ; les sources de capture prennent leur image dans `frames`.
    pub fn render_with(
        &mut self,
        canvas: Canvas,
        scene: &Scene,
        frame: u64,
        frames: &dyn FrameProvider,
        out: &mut Vec<u8>,
    ) {
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
            self.draw_source(canvas, source, frame, frames, out);
        }
    }

    pub fn render_image(&mut self, canvas: Canvas, scene: &Scene, frame: u64) -> RgbaImage {
        self.render_image_with(canvas, scene, frame, &NoFrames)
    }

    pub fn render_image_with(
        &mut self,
        canvas: Canvas,
        scene: &Scene,
        frame: u64,
        frames: &dyn FrameProvider,
    ) -> RgbaImage {
        let mut buf = Vec::new();
        self.render_with(canvas, scene, frame, frames, &mut buf);
        RgbaImage::from_raw(canvas.width, canvas.height, buf).expect("taille cohérente")
    }

    fn draw_source(
        &mut self,
        canvas: Canvas,
        source: &Source,
        frame: u64,
        frames: &dyn FrameProvider,
        out: &mut [u8],
    ) {
        let t = source.transform;
        let (cw, ch) = (canvas.width as i64, canvas.height as i64);
        let x0 = (t.x.round() as i64).clamp(0, cw);
        let y0 = (t.y.round() as i64).clamp(0, ch);
        let x1 = ((t.x + t.width).round() as i64).clamp(0, cw);
        let y1 = ((t.y + t.height).round() as i64).clamp(0, ch);
        if x1 <= x0 || y1 <= y0 || t.width <= 0.0 || t.height <= 0.0 {
            return;
        }
        // L'Arc garde l'image de capture vivante pendant le dessin, même si une
        // nouvelle image arrive entre-temps.
        let live;
        let pixels = match &source.kind {
            SourceKind::Color { rgba } => Pixels::Solid(*rgba),
            SourceKind::TestPattern => Pixels::Pattern,
            SourceKind::Image { path } => {
                let entry = self
                    .images
                    .entry(path.clone())
                    .or_insert_with(|| image::open(path).ok().map(|i| i.to_rgba8()));
                match entry {
                    Some(img) => Pixels::Raster {
                        width: img.width(),
                        height: img.height(),
                        data: img.as_raw(),
                    },
                    None => return, // Fichier illisible : on ignore la source.
                }
            }
            kind if kind.is_live_video() => match frames.frame(source.id) {
                Some(f) => {
                    live = f;
                    Pixels::Raster {
                        width: live.width,
                        height: live.height,
                        data: &live.data,
                    }
                }
                // Pas encore d'image (démarrage, erreur) : rien à dessiner.
                None => return,
            },
            // Sources purement audio : rien à dessiner.
            _ => return,
        };
        // Colonne source de chaque pixel du rectangle, calculée une fois par source.
        let columns: Vec<usize> = match pixels {
            Pixels::Raster { width, .. } => (x0..x1)
                .map(|x| {
                    let u = ((x as f32 + 0.5) - t.x) / t.width;
                    ((u * width as f32) as usize).min(width as usize - 1)
                })
                .collect(),
            _ => Vec::new(),
        };
        let opaque = t.opacity >= 1.0;
        for y in y0..y1 {
            // Coordonnées normalisées dans la source (0..1).
            let v = ((y as f32 + 0.5) - t.y) / t.height;
            let row = ((y * cw + x0) * 4) as usize..((y * cw + x1) * 4) as usize;
            let dst_row = &mut out[row];
            match &pixels {
                Pixels::Solid(px) => {
                    for d in dst_row.chunks_exact_mut(4) {
                        blend(d, *px, t.opacity);
                    }
                }
                Pixels::Pattern => {
                    for (i, d) in dst_row.chunks_exact_mut(4).enumerate() {
                        let u = (((x0 + i as i64) as f32 + 0.5) - t.x) / t.width;
                        blend(d, test_pattern(u, v, frame), t.opacity);
                    }
                }
                Pixels::Raster {
                    width,
                    height,
                    data,
                } => {
                    let sy = ((v * *height as f32) as usize).min(*height as usize - 1);
                    let src_row = &data[sy * *width as usize * 4..(sy + 1) * *width as usize * 4];
                    for (d, &sx) in dst_row.chunks_exact_mut(4).zip(&columns) {
                        let s = &src_row[sx * 4..sx * 4 + 4];
                        if opaque && s[3] == 255 {
                            d.copy_from_slice(s);
                        } else {
                            blend(d, [s[0], s[1], s[2], s[3]], t.opacity);
                        }
                    }
                }
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
            volume: 1.0,
            muted: false,
        });
        let img = Compositor::new().render_image(canvas, &scene, 0);
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(img.get_pixel(3, 1).0, [255, 0, 0, 255]);
    }

    fn live_source(kind: SourceKind, transform: Transform) -> Source {
        Source {
            id: Uuid::new_v4(),
            name: "capture".into(),
            kind,
            transform,
            visible: true,
            volume: 1.0,
            muted: false,
        }
    }

    /// Image 2x2 : rouge, vert / bleu, blanc.
    fn quad() -> Arc<VideoFrame> {
        Arc::new(
            VideoFrame::new(
                2,
                2,
                vec![
                    255, 0, 0, 255, 0, 255, 0, 255, //
                    0, 0, 255, 255, 255, 255, 255, 255,
                ],
            )
            .unwrap(),
        )
    }

    #[test]
    fn frame_provider_scaled_into_transform() {
        let canvas = Canvas {
            width: 8,
            height: 6,
            fps: 30,
        };
        let mut scene = Scene::new("t");
        let src = live_source(
            SourceKind::Camera {
                device_id: None,
                width: None,
                height: None,
                fps: None,
            },
            Transform {
                x: 2.0,
                y: 1.0,
                width: 4.0,
                height: 4.0,
                opacity: 1.0,
            },
        );
        let id = src.id;
        scene.sources.push(src);
        let frame = quad();
        let provider = move |sid: Uuid| (sid == id).then(|| Arc::clone(&frame));
        let img = Compositor::new().render_image_with(canvas, &scene, 0, &provider);
        // Hors du rectangle : fond noir.
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(img.get_pixel(6, 1).0, [0, 0, 0, 255]);
        assert_eq!(img.get_pixel(2, 5).0, [0, 0, 0, 255]);
        // Image 2x2 agrandie en 4x4 : chaque pixel source couvre 2x2 pixels.
        for (x, y, px) in [
            (2, 1, [255, 0, 0, 255]),
            (3, 2, [255, 0, 0, 255]),
            (4, 1, [0, 255, 0, 255]),
            (5, 2, [0, 255, 0, 255]),
            (2, 3, [0, 0, 255, 255]),
            (3, 4, [0, 0, 255, 255]),
            (4, 3, [255, 255, 255, 255]),
            (5, 4, [255, 255, 255, 255]),
        ] {
            assert_eq!(img.get_pixel(x, y).0, px, "pixel ({x}, {y})");
        }
    }

    #[test]
    fn frame_provider_downscale_and_opacity() {
        let canvas = Canvas {
            width: 2,
            height: 1,
            fps: 30,
        };
        // Image 4x2 réduite à 2x1, à moitié transparente.
        let data: Vec<u8> = (0..8)
            .flat_map(|i| {
                if i % 4 < 2 {
                    [200, 0, 0, 255]
                } else {
                    [0, 200, 0, 255]
                }
            })
            .collect();
        let frame = Arc::new(VideoFrame::new(4, 2, data).unwrap());
        let mut scene = Scene::new("t");
        scene.sources.push(live_source(
            SourceKind::MediaFile {
                path: "x.mp4".into(),
                looping: false,
            },
            Transform {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 1.0,
                opacity: 0.5,
            },
        ));
        let provider = move |_| Some(Arc::clone(&frame));
        let img = Compositor::new().render_image_with(canvas, &scene, 0, &provider);
        assert_eq!(img.get_pixel(0, 0).0, [100, 0, 0, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [0, 100, 0, 255]);
    }

    #[test]
    fn live_source_without_frame_draws_nothing() {
        let canvas = Canvas {
            width: 2,
            height: 2,
            fps: 30,
        };
        let mut scene = Scene::new("t");
        scene.sources.push(live_source(
            SourceKind::SrtInput {
                url: "srt://h:1".into(),
            },
            Transform::full(canvas),
        ));
        scene.sources.push(live_source(
            SourceKind::AudioInput { device_id: None },
            Transform::full(canvas),
        ));
        let img = Compositor::new().render_image(canvas, &scene, 0);
        assert!(img.pixels().all(|p| p.0 == [0, 0, 0, 255]));
        assert!(VideoFrame::new(2, 2, vec![0; 3]).is_none());
    }

    #[test]
    fn opacity_blends() {
        let mut px = [0, 0, 0, 255];
        blend(&mut px, [200, 100, 0, 255], 0.5);
        assert_eq!(px, [100, 50, 0, 255]);
    }
}
