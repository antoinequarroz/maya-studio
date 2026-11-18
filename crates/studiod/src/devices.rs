//! Découverte des périphériques d'entrée (caméras, micros) avec `gst::DeviceMonitor`.
//!
//! Le même code sert sur toutes les plateformes : les fournisseurs de
//! périphériques (v4l2 / PipeWire / PulseAudio sous Linux, AVFoundation sous
//! macOS, Media Foundation / WASAPI sous Windows) sont des plugins GStreamer
//! chargés à l'exécution. Sans périphérique ni fournisseur, les listes sont vides.

use gstreamer as gst;
use gstreamer::prelude::*;
use serde::Serialize;

/// Classe GStreamer des sources vidéo.
pub const VIDEO_CLASS: &str = "Video/Source";
/// Classe GStreamer des sources audio.
pub const AUDIO_CLASS: &str = "Audio/Source";

/// Propriétés qui identifient un périphérique de façon stable, par ordre de préférence.
const ID_KEYS: &[&str] = &[
    "device.path",
    "api.v4l2.path",
    "object.path",
    "node.name",
    "device.id",
    "device.strid",
    "avf.unique_id",
    "unique-id",
];

/// Une entrée de la réponse `GetInputDevices`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InputDevice {
    /// Identifiant à passer en `device_id` d'une source `camera` ou `audio_input`.
    pub id: String,
    /// Nom lisible.
    pub name: String,
    /// Résumé des formats proposés, par exemple « video/x-raw (YUY2), image/jpeg · jusqu'à 1920x1080 ».
    pub caps_summary: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct DeviceList {
    pub video: Vec<InputDevice>,
    pub audio: Vec<InputDevice>,
}

/// Identifiant stable d'un périphérique : une propriété connue (chemin, nœud…)
/// ou, à défaut, son nom affiché.
pub fn device_id(device: &gst::Device) -> String {
    if let Some(props) = device.properties() {
        for key in ID_KEYS {
            if let Ok(v) = props.get::<String>(*key) {
                if !v.is_empty() {
                    return v;
                }
            }
            // Certaines plateformes publient des identifiants numériques.
            if let Ok(v) = props.get::<u64>(*key) {
                return v.to_string();
            }
            if let Ok(v) = props.get::<i32>(*key) {
                return v.to_string();
            }
        }
    }
    device.display_name().to_string()
}

/// Résumé lisible des caps d'un périphérique.
pub fn caps_summary(caps: &gst::Caps) -> String {
    if caps.is_any() {
        return "tous formats".into();
    }
    let mut kinds: Vec<String> = Vec::new();
    let mut max_px: Option<(i32, i32)> = None;
    let mut max_rate: Option<i32> = None;
    let mut max_channels: Option<i32> = None;
    for s in caps.iter() {
        let mut kind = s.name().to_string();
        if let Ok(f) = s.get::<String>("format") {
            kind = format!("{kind} ({f})");
        }
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
        if let (Some(w), Some(h)) = (max_int(s, "width"), max_int(s, "height")) {
            if max_px.is_none_or(|(mw, mh)| w * h > mw * mh) {
                max_px = Some((w, h));
            }
        }
        if let Some(r) = max_int(s, "rate") {
            max_rate = Some(max_rate.map_or(r, |m| m.max(r)));
        }
        if let Some(c) = max_int(s, "channels") {
            max_channels = Some(max_channels.map_or(c, |m| m.max(c)));
        }
    }
    const MAX_KINDS: usize = 4;
    let extra = kinds.len().saturating_sub(MAX_KINDS);
    kinds.truncate(MAX_KINDS);
    let mut summary = kinds.join(", ");
    if extra > 0 {
        summary.push_str(&format!(" (+{extra})"));
    }
    if let Some((w, h)) = max_px {
        summary.push_str(&format!(" · jusqu'à {w}x{h}"));
    }
    if let Some(c) = max_channels {
        summary.push_str(&format!(" · {c} canaux"));
    }
    if let Some(r) = max_rate {
        summary.push_str(&format!(" · {r} Hz"));
    }
    summary
}

/// Valeur entière maximale d'un champ (entier fixe, intervalle ou liste).
fn max_int(s: &gst::StructureRef, field: &str) -> Option<i32> {
    if let Ok(v) = s.get::<i32>(field) {
        return Some(v);
    }
    if let Ok(r) = s.get::<gst::IntRange<i32>>(field) {
        return Some(r.max());
    }
    if let Ok(list) = s.get::<gst::List>(field) {
        return list.iter().filter_map(|v| v.get::<i32>().ok()).max();
    }
    None
}

fn monitor_devices(class: &str) -> Vec<gst::Device> {
    if let Err(e) = gst::init() {
        tracing::warn!(error = %e, "GStreamer indisponible : aucun périphérique");
        return Vec::new();
    }
    let monitor = gst::DeviceMonitor::new();
    if monitor.add_filter(Some(class), None).is_none() {
        return Vec::new();
    }
    if let Err(e) = monitor.start() {
        // Aucun fournisseur utilisable (conteneur, serveur audio absent…) : pas une erreur.
        tracing::debug!(class, error = %e, "surveillance des périphériques impossible");
        return Vec::new();
    }
    let devices: Vec<gst::Device> = monitor.devices().into_iter().collect();
    monitor.stop();
    devices
}

fn describe(devices: &[gst::Device]) -> Vec<InputDevice> {
    let mut out: Vec<InputDevice> = Vec::new();
    for d in devices {
        let id = device_id(d);
        // Un même périphérique peut être annoncé par deux fournisseurs (v4l2 et PipeWire).
        if out.iter().any(|o| o.id == id) {
            continue;
        }
        out.push(InputDevice {
            id,
            name: d.display_name().to_string(),
            caps_summary: d.caps().map(|c| caps_summary(&c)).unwrap_or_default(),
        });
    }
    out
}

/// Périphériques d'entrée présents (requête `GetInputDevices`). Ne renvoie jamais
/// d'erreur : sans périphérique ni fournisseur, les listes sont vides.
pub fn list() -> DeviceList {
    DeviceList {
        video: describe(&monitor_devices(VIDEO_CLASS)),
        audio: describe(&monitor_devices(AUDIO_CLASS)),
    }
}

/// Retrouve un périphérique par son identifiant (ou son nom affiché).
pub fn find(class: &str, id: &str) -> Option<gst::Device> {
    let devices = monitor_devices(class);
    devices
        .iter()
        .find(|d| device_id(d) == id)
        .or_else(|| devices.iter().find(|d| d.display_name() == id))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_of_video_and_audio_caps() {
        gst::init().unwrap();
        let caps: gst::Caps =
            "video/x-raw, format=YUY2, width=640, height=480; image/jpeg, width=1920, height=1080; \
             video/x-raw, format=YUY2, width=[ 1, 1280 ], height=[ 1, 720 ]"
                .parse()
                .unwrap();
        assert_eq!(
            caps_summary(&caps),
            "video/x-raw (YUY2), image/jpeg · jusqu'à 1920x1080"
        );
        let caps: gst::Caps = "audio/x-raw, format=S16LE, rate=48000, channels=2"
            .parse()
            .unwrap();
        assert_eq!(
            caps_summary(&caps),
            "audio/x-raw (S16LE) · 2 canaux · 48000 Hz"
        );
        assert_eq!(caps_summary(&gst::Caps::new_any()), "tous formats");
    }

    #[test]
    fn listing_never_fails() {
        // Dans un conteneur sans caméra ni micro, les listes peuvent être vides.
        let list = list();
        for d in list.video.iter().chain(&list.audio) {
            assert!(!d.id.is_empty());
        }
        assert!(find(VIDEO_CLASS, "périphérique-inexistant").is_none());
    }
}
