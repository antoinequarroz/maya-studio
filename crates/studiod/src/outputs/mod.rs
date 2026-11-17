//! Sorties du studio : enregistrement fichier et diffusion RTMP/SRT.
//!
//! Deux backends derrière le même trait [`Output`] :
//! - `gstreamer` (par défaut) : pipeline in-process, encodeurs matériels choisis
//!   à l'exécution, piste audio AAC ; voir [`gst`] et [`crate::encoders`] ;
//! - `ffmpeg` (provisoire, développement seulement) : processus `ffmpeg` externe
//!   avec `libx264` (GPL), voir [`ffmpeg`].

pub mod ffmpeg;
#[cfg(feature = "gstreamer")]
pub mod gst;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use studio_core::Canvas;

/// Fréquence d'échantillonnage de la piste audio de toutes les sorties.
pub const AUDIO_RATE: u32 = 48_000;
/// Nombre de canaux (stéréo entrelacée).
pub const AUDIO_CHANNELS: u32 = 2;
/// Débits vidéo par défaut (kbit/s).
pub const DEFAULT_RECORD_BITRATE_KBPS: u32 = 6000;
pub const DEFAULT_STREAM_BITRATE_KBPS: u32 = 4500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Record,
    Stream,
}

/// Conteneur d'enregistrement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordFormat {
    /// MP4 (mp4mux) ; finalisé à l'arrêt, ou fragmenté pour résister aux plantages.
    #[default]
    Mp4,
    /// Matroska (matroskamux) : lisible même si le processus s'arrête brutalement.
    Mkv,
}

impl RecordFormat {
    pub fn extension(self) -> &'static str {
        match self {
            RecordFormat::Mp4 => "mp4",
            RecordFormat::Mkv => "mkv",
        }
    }

    /// Déduit le format de l'extension d'un chemin (MP4 par défaut).
    pub fn from_path(path: &str) -> Self {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".mkv") || lower.ends_with(".mka") {
            RecordFormat::Mkv
        } else {
            RecordFormat::Mp4
        }
    }
}

/// Protocole de diffusion, déduit de l'URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamProtocol {
    /// `rtmp://` ou `rtmps://` (FLV).
    Rtmp,
    /// `srt://` (MPEG-TS).
    Srt,
}

impl StreamProtocol {
    pub fn from_url(url: &str) -> Option<Self> {
        let lower = url.trim().to_ascii_lowercase();
        if lower.starts_with("rtmp://") || lower.starts_with("rtmps://") {
            Some(StreamProtocol::Rtmp)
        } else if lower.starts_with("srt://") {
            Some(StreamProtocol::Srt)
        } else {
            None
        }
    }
}

/// Destination affichable dans les journaux : la clé de diffusion RTMP (dernier
/// segment du chemin) et les paramètres SRT (`passphrase`, `streamid`…) sont masqués.
pub fn redact_target(target: &str) -> String {
    match StreamProtocol::from_url(target) {
        Some(StreamProtocol::Rtmp) => match target.rfind('/') {
            Some(i) if i > target.find("://").map_or(0, |p| p + 2) => {
                format!("{}/***", &target[..i])
            }
            _ => target.to_string(),
        },
        Some(StreamProtocol::Srt) => match target.split_once('?') {
            Some((base, _)) => format!("{base}?***"),
            None => target.to_string(),
        },
        None => target.to_string(),
    }
}

/// Backend de sortie choisi au lancement de `studiod`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Gstreamer,
    Ffmpeg,
}

impl std::str::FromStr for Backend {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "gstreamer" | "gst" => Ok(Backend::Gstreamer),
            "ffmpeg" => Ok(Backend::Ffmpeg),
            other => Err(format!(
                "backend inconnu : {other} (attendu : gstreamer ou ffmpeg)"
            )),
        }
    }
}

/// Encodeur H.264 demandé : `auto` ou un nom d'élément GStreamer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncoderChoice {
    Auto,
    Named(String),
}

impl std::str::FromStr for EncoderChoice {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("nom d'encodeur vide".into());
        }
        Ok(if s.eq_ignore_ascii_case("auto") {
            EncoderChoice::Auto
        } else {
            EncoderChoice::Named(s.to_string())
        })
    }
}

/// Tout ce qu'il faut pour démarrer une sortie.
#[derive(Debug, Clone)]
pub struct OutputSettings {
    pub kind: OutputKind,
    pub canvas: Canvas,
    /// Chemin du fichier ou URL de diffusion.
    pub target: String,
    /// Conteneur (enregistrement seulement).
    pub format: RecordFormat,
    /// MP4 fragmenté (enregistrement MP4 seulement).
    pub fragmented: bool,
    pub bitrate_kbps: u32,
    pub encoder: EncoderChoice,
}

/// Point d'entrée audio d'une sortie : échantillons S16 entrelacés stéréo à 48 kHz.
///
/// Partageable entre threads : le moteur y pousse aujourd'hui du silence,
/// le mixeur audio y poussera le mix micro/sources avec la même interface.
pub trait AudioSink: Send + Sync {
    /// Ajoute des échantillons à la suite des précédents (horodatage par compteur
    /// d'échantillons). Ne bloque jamais : si la file est pleine, le bloc est perdu.
    fn push_audio(&self, interleaved: &[i16]) -> std::io::Result<()>;
}

/// Une sortie qui consomme les images programme.
pub trait Output: Send {
    /// Pousse une image RGBA. Ne doit pas bloquer la boucle de rendu.
    /// Une erreur arrête la sortie.
    fn push_frame(&mut self, rgba: &[u8]) -> std::io::Result<()>;
    /// Entrée audio de la sortie, si le backend en a une.
    fn audio_sink(&self) -> Option<Arc<dyn AudioSink>>;
    /// Arrête proprement (finalise le fichier, ferme la connexion).
    /// `Err` si la finalisation n'a pas pu se faire complètement.
    fn stop(self: Box<Self>) -> Result<(), String>;
    /// Destination : chemin du fichier ou URL de diffusion.
    fn target(&self) -> &str;
    /// Encodeur vidéo utilisé.
    fn encoder(&self) -> &str;
    /// Images perdues parce que l'encodeur ne suivait pas.
    fn dropped_frames(&self) -> u64;
}

/// Nombre d'échantillons audio (par canal) correspondant à l'image vidéo `frame`
/// (comptée depuis 0) à `fps` images/s. La somme sur une seconde vaut exactement
/// [`AUDIO_RATE`] : l'audio reste calé sur les horodatages vidéo.
pub fn audio_samples_for_frame(frame: u64, fps: u32) -> usize {
    let fps = u64::from(fps.max(1));
    let rate = u64::from(AUDIO_RATE);
    let start = frame * rate / fps;
    let end = (frame + 1) * rate / fps;
    (end - start) as usize
}

/// Démarre une sortie avec le backend demandé.
pub fn start(backend: Backend, settings: &OutputSettings) -> Result<Box<dyn Output>, String> {
    match backend {
        Backend::Ffmpeg => ffmpeg::FfmpegOutput::start(settings)
            .map(|o| Box::new(o) as Box<dyn Output>)
            .map_err(|e| format!("ffmpeg : {e}")),
        #[cfg(feature = "gstreamer")]
        Backend::Gstreamer => gst::GstOutput::start(settings).map(|o| Box::new(o) as Box<dyn Output>),
        #[cfg(not(feature = "gstreamer"))]
        Backend::Gstreamer => Err(
            "studiod a été compilé sans la fonctionnalité « gstreamer » ; relance avec --output-backend ffmpeg"
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_follows_video_timestamps() {
        for fps in [24, 25, 30, 50, 60, 120, 7] {
            let total: usize = (0..u64::from(fps))
                .map(|f| audio_samples_for_frame(f, fps))
                .sum();
            assert_eq!(total, AUDIO_RATE as usize, "fps {fps}");
        }
        assert_eq!(audio_samples_for_frame(0, 30), 1600);
    }

    #[test]
    fn formats_and_urls() {
        assert_eq!(RecordFormat::from_path("a/b.MKV"), RecordFormat::Mkv);
        assert_eq!(RecordFormat::from_path("a/b.mp4"), RecordFormat::Mp4);
        assert_eq!(RecordFormat::from_path("sans-extension"), RecordFormat::Mp4);
        assert_eq!(
            StreamProtocol::from_url("rtmps://live.example/app/cle"),
            Some(StreamProtocol::Rtmp)
        );
        assert_eq!(
            StreamProtocol::from_url("srt://127.0.0.1:9000"),
            Some(StreamProtocol::Srt)
        );
        assert_eq!(StreamProtocol::from_url("http://x"), None);
        assert_eq!(
            redact_target("rtmp://live.example/app/ma-cle"),
            "rtmp://live.example/app/***"
        );
        assert_eq!(redact_target("rtmp://live.example"), "rtmp://live.example");
        assert_eq!(
            redact_target("srt://h:9000?passphrase=secret"),
            "srt://h:9000?***"
        );
        assert_eq!(redact_target("rec/a.mp4"), "rec/a.mp4");
        assert_eq!("ffmpeg".parse::<Backend>().unwrap(), Backend::Ffmpeg);
        assert!("vlc".parse::<Backend>().is_err());
    }
}
