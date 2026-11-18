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
/// Les sources de capture (caméra, écran, fenêtre, micro, fichier média, SRT
/// entrant) sont alimentées par le moteur (`studiod`, via GStreamer) ; le cœur
/// ne fait que les décrire et dessiner les images qu'on lui fournit
/// (voir [`crate::compositor::FrameProvider`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceKind {
    /// Aplat de couleur RGBA.
    Color { rgba: [u8; 4] },
    /// Image fixe depuis un fichier (PNG ou JPEG).
    Image { path: String },
    /// Mire animée, utile pour vérifier qu'une sortie est vivante.
    TestPattern,
    /// Caméra (webcam, carte d'acquisition). Sans `device_id`, la caméra par défaut.
    Camera {
        /// Identifiant renvoyé par `GetInputDevices`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_id: Option<String>,
        /// Taille et cadence souhaitées ; l'image est mise à l'échelle si la
        /// caméra ne propose pas ce mode.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fps: Option<u32>,
    },
    /// Capture d'un écran entier. Sans `display_id`, l'écran principal.
    DisplayCapture {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_id: Option<String>,
    },
    /// Capture d'une fenêtre (selon ce que permet la plateforme).
    WindowCapture {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window_id: Option<String>,
    },
    /// Entrée audio (micro, interface audio). Sans `device_id`, l'entrée par défaut.
    AudioInput {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_id: Option<String>,
    },
    /// Fichier vidéo ou audio (chemin local ou URI), lu en temps réel.
    MediaFile {
        path: String,
        /// Relire depuis le début à la fin du fichier.
        #[serde(rename = "loop", default)]
        looping: bool,
    },
    /// Flux SRT entrant (`srt://hôte:port`, `?mode=listener` pour attendre l'émetteur).
    SrtInput { url: String },
}

impl SourceKind {
    /// Vrai pour les sources dont les images viennent d'une capture du moteur.
    pub fn is_live_video(&self) -> bool {
        matches!(
            self,
            SourceKind::Camera { .. }
                | SourceKind::DisplayCapture { .. }
                | SourceKind::WindowCapture { .. }
                | SourceKind::MediaFile { .. }
                | SourceKind::SrtInput { .. }
        )
    }

    /// Vrai pour les sources qui peuvent produire du son.
    pub fn has_audio(&self) -> bool {
        matches!(
            self,
            SourceKind::AudioInput { .. }
                | SourceKind::MediaFile { .. }
                | SourceKind::SrtInput { .. }
        )
    }

    /// Vrai pour les sources qui nécessitent un pipeline de capture.
    pub fn is_capture(&self) -> bool {
        self.is_live_video() || self.has_audio()
    }
}

/// Volume maximal d'une source (gain linéaire, +6 dB).
pub const MAX_VOLUME: f32 = 2.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Source {
    pub id: Uuid,
    pub name: String,
    #[serde(flatten)]
    pub kind: SourceKind,
    pub transform: Transform,
    pub visible: bool,
    /// Gain audio linéaire, de 0.0 à [`MAX_VOLUME`] (1.0 = inchangé).
    #[serde(default = "one")]
    pub volume: f32,
    /// Son coupé.
    #[serde(default)]
    pub muted: bool,
    /// Métadonnées libres du client qui a créé la source (objet JSON, voir
    /// [`MAX_META_BYTES`]). Le studio les conserve sans les interpréter : Nino s'en
    /// sert pour reconnaître ses propres sources (médias, invités).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

/// Taille maximale des métadonnées d'une source ou d'une entrée audio, en JSON.
pub const MAX_META_BYTES: usize = 4096;

/// Entrée audio globale du studio (domaine `studio.mixer`) : un micro qui ne
/// dépend d'aucune scène, comme une tranche de console. Elle part toujours dans
/// le mix (sauf si elle est coupée), quelle que soit la scène à l'antenne.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioInput {
    pub id: Uuid,
    pub name: String,
    /// Toujours `audio_input` pour l'instant ; d'autres types pourront suivre.
    #[serde(flatten)]
    pub kind: SourceKind,
    /// Gain audio linéaire, de 0.0 à [`MAX_VOLUME`] (1.0 = inchangé).
    #[serde(default = "one")]
    pub volume: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
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
