//! Modèle de données du studio : canvas, scènes, sources et état de sortie.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Taille et cadence de la sortie programme.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            fps: 30,
        }
    }
}

/// Position et taille d'une source dans le canvas, en pixels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Opacité de 0.0 (invisible) à 1.0 (opaque).
    #[serde(default = "one")]
    pub opacity: f32,
}

fn one() -> f32 {
    1.0
}

impl Transform {
    pub fn full(canvas: Canvas) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: canvas.width as f32,
            height: canvas.height as f32,
            opacity: 1.0,
        }
    }
}

/// Type de source et ses réglages.
///
/// Les captures réelles (caméra, écran, micro, NDI, SRT entrant) arriveront
/// avec le backend GStreamer ; ce sont des variantes supplémentaires de cet enum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceKind {
    /// Aplat de couleur RGBA.
    Color { rgba: [u8; 4] },
    /// Image fixe depuis un fichier (PNG ou JPEG).
    Image { path: String },
    /// Mire animée, utile pour vérifier qu'une sortie est vivante.
    TestPattern,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Source {
    pub id: Uuid,
    pub name: String,
    #[serde(flatten)]
    pub kind: SourceKind,
    pub transform: Transform,
    pub visible: bool,
}

/// Une scène est une pile ordonnée de sources : la première est dessinée en dessous.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Scene {
    pub id: Uuid,
    pub name: String,
    pub sources: Vec<Source>,
}

impl Scene {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            sources: Vec::new(),
        }
    }
}
