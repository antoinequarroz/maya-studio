//! Choix et réglage des encodeurs (H.264 vidéo, AAC audio) pour le backend GStreamer.
//!
//! Les éléments sont cherchés à l'exécution avec `gst::ElementFactory::find` :
//! aucun crate propre à une plateforme n'est nécessaire, le même binaire
//! utilise VideoToolbox sur macOS, NVENC / QuickSync / AMF / Media Foundation
//! sur Windows et NVENC / VA-API sur Linux quand ils sont présents.
//!
//! Licences : `openh264enc` (BSD, binaire Cisco) est le repli logiciel par défaut.
//! `x264enc` est sous GPL : il n'est jamais choisi automatiquement et n'est
//! utilisé que s'il est demandé explicitement (`--encoder x264enc`).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use gstreamer as gst;
use gstreamer::glib;
use gstreamer::prelude::*;
use serde::Serialize;

/// Repli logiciel par défaut (licence BSD).
pub const SOFTWARE_FALLBACK: &str = "openh264enc";
/// Encodeur GPL, uniquement sur demande explicite.
pub const GPL_ENCODER: &str = "x264enc";
/// Encodeurs AAC, par ordre de préférence.
pub const AAC_CANDIDATES: &[&str] = &["avenc_aac", "voaacenc", "fdkaacenc"];

/// Encodeurs matériels connus, par plateforme et par ordre de préférence.
pub fn hardware_candidates(os: &str) -> &'static [&'static str] {
    match os {
        "macos" => &["vtenc_h264_hw", "vtenc_h264"],
        "windows" => &["nvh264enc", "qsvh264enc", "amfh264enc", "mfh264enc"],
        "linux" => &["nvh264enc", "vah264enc", "vaapih264enc"],
        _ => &[],
    }
}

/// Candidats essayés en mode automatique : matériels d'abord, puis le repli logiciel.
/// `x264enc` n'y figure jamais.
pub fn auto_candidates(os: &str) -> Vec<&'static str> {
    let mut v = hardware_candidates(os).to_vec();
    v.push(SOFTWARE_FALLBACK);
    v
}

/// Liste affichée par `GetEncoders` : les candidats automatiques puis `x264enc`.
pub fn listed_candidates(os: &str) -> Vec<&'static str> {
    let mut v = auto_candidates(os);
    v.push(GPL_ENCODER);
    v
}

/// Vrai pour un encodeur matériel (table connue ou métadonnée « Hardware » de la fabrique).
pub fn is_hardware(name: &str) -> bool {
    if name == SOFTWARE_FALLBACK || name == GPL_ENCODER {
        return false;
    }
    let known = ["macos", "windows", "linux"]
        .iter()
        .any(|os| hardware_candidates(os).contains(&name));
    known
        || gst::ElementFactory::find(name)
            .and_then(|f| f.metadata("klass").map(|k| k.contains("Hardware")))
            .unwrap_or(false)
}

pub use crate::outputs::EncoderChoice;

/// Une entrée de la réponse `GetEncoders`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EncoderInfo {
    pub name: String,
    pub hardware: bool,
    pub available: bool,
    pub selected: bool,
    /// Vrai pour un encodeur sous GPL (`x264enc`), à éviter en distribution.
    pub gpl: bool,
}

fn probe_cache() -> &'static Mutex<HashMap<String, bool>> {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Vérifie qu'un encodeur existe et s'initialise (passage rapide à l'état READY).
/// Le résultat est mis en cache pour la durée du processus.
pub fn probe(name: &str) -> bool {
    if let Some(v) = probe_cache().lock().unwrap().get(name) {
        return *v;
    }
    let ok = probe_uncached(name);
    probe_cache().lock().unwrap().insert(name.to_string(), ok);
    ok
}

fn probe_uncached(name: &str) -> bool {
    if gst::init().is_err() {
        return false;
    }
    let Some(factory) = gst::ElementFactory::find(name) else {
        return false;
    };
    let Ok(element) = factory.create().build() else {
        return false;
    };
    let ok = element.set_state(gst::State::Ready).is_ok();
    let _ = element.set_state(gst::State::Null);
    if !ok {
        tracing::debug!(encoder = name, "encodeur présent mais inutilisable");
    }
    ok
}

/// Choisit l'encodeur H.264 à utiliser.
pub fn select(choice: &EncoderChoice) -> Result<String, String> {
    select_with(choice, std::env::consts::OS, probe)
}

/// Logique de choix, avec la fonction de test injectable (tests unitaires).
pub fn select_with(
    choice: &EncoderChoice,
    os: &str,
    available: impl Fn(&str) -> bool,
) -> Result<String, String> {
    match choice {
        EncoderChoice::Auto => auto_candidates(os)
            .into_iter()
            .find(|n| available(n))
            .map(str::to_string)
            .ok_or_else(|| {
                format!(
                    "aucun encodeur H.264 disponible (essayés : {}) ; installe gst-plugins-bad (openh264)",
                    auto_candidates(os).join(", ")
                )
            }),
        EncoderChoice::Named(name) => {
            if !available(name) {
                return Err(format!("encodeur « {name} » introuvable ou inutilisable"));
            }
            static GPL_WARNING: std::sync::Once = std::sync::Once::new();
            if name == GPL_ENCODER {
                GPL_WARNING.call_once(|| tracing::warn!(
                    "x264enc est sous licence GPL : à réserver au développement, \
                     ne pas distribuer Maya Studio avec cet encodeur"
                ));
            }
            Ok(name.clone())
        }
    }
}

/// État de tous les encodeurs connus, pour `GetEncoders`.
pub fn list(choice: &EncoderChoice) -> Vec<EncoderInfo> {
    let os = std::env::consts::OS;
    let selected = select(choice).ok();
    let mut names: Vec<String> = listed_candidates(os)
        .into_iter()
        .map(str::to_string)
        .collect();
    if let EncoderChoice::Named(n) = choice {
        if !names.contains(n) {
            names.push(n.clone());
        }
    }
    names
        .into_iter()
        .map(|name| EncoderInfo {
            hardware: is_hardware(&name),
            available: probe(&name),
            selected: selected.as_deref() == Some(name.as_str()),
            gpl: name == GPL_ENCODER,
            name,
        })
        .collect()
}

/// Premier encodeur AAC présent.
pub fn select_aac() -> Result<String, String> {
    select_aac_with(|n| gst::ElementFactory::find(n).is_some())
}

pub fn select_aac_with(exists: impl Fn(&str) -> bool) -> Result<String, String> {
    AAC_CANDIDATES
        .iter()
        .find(|n| exists(n))
        .map(|n| n.to_string())
        .ok_or_else(|| {
            format!(
                "aucun encodeur AAC disponible (essayés : {})",
                AAC_CANDIDATES.join(", ")
            )
        })
}

/// Valeur de propriété à appliquer si l'élément la propose.
#[derive(Debug, Clone, PartialEq)]
pub enum PropValue {
    Uint(u64),
    Bool(bool),
    /// Pseudo (nick) d'énumération ou de drapeaux, ou chaîne ; plusieurs essais possibles.
    Nick(&'static [&'static str]),
}

/// Réglages H.264 : débit, image clé toutes les 2 s, faible latence.
/// Chaque propriété n'est appliquée que si l'élément l'a (voir [`apply`]).
pub fn video_tuning(encoder: &str, bitrate_kbps: u32, fps: u32) -> Vec<(&'static str, PropValue)> {
    use PropValue::*;
    let gop = u64::from(fps.max(1)) * 2;
    // openh264enc attend des bits/s, les autres des kbit/s.
    let bitrate = if encoder == "openh264enc" {
        u64::from(bitrate_kbps) * 1000
    } else {
        u64::from(bitrate_kbps)
    };
    vec![
        ("bitrate", Uint(bitrate)),
        // Intervalle d'images clés (le nom varie selon l'élément).
        ("gop-size", Uint(gop)),
        ("key-int-max", Uint(gop)),
        ("keyframe-period", Uint(gop)),
        ("max-keyframe-interval", Uint(gop)),
        ("max-keyframe-interval-duration", Uint(2_000_000_000)),
        // Débit constant.
        ("rate-control", Nick(&["cbr", "bitrate"])),
        ("rc-mode", Nick(&["cbr"])),
        // Faible latence.
        ("zerolatency", Bool(true)),
        ("low-latency", Bool(true)),
        ("realtime", Bool(true)),
        ("allow-frame-reordering", Bool(false)),
        ("tune", Nick(&["zerolatency"])),
        ("speed-preset", Nick(&["veryfast"])),
        ("usage-type", Nick(&["camera"])),
        ("usage", Nick(&["low-latency", "ultra-low-latency"])),
        ("preset", Nick(&["low-latency-hq", "low-latency"])),
        ("bframes", Uint(0)),
        ("b-frames", Uint(0)),
    ]
}

/// Réglages AAC : 160 kbit/s (toutes les implémentations attendent des bits/s).
pub fn audio_tuning() -> Vec<(&'static str, PropValue)> {
    vec![("bitrate", PropValue::Uint(160_000))]
}

/// Applique les réglages existants sur l'élément ; les autres sont ignorés.
pub fn apply(element: &gst::Element, tuning: &[(&str, PropValue)]) {
    for (name, value) in tuning {
        if set_if_exists(element, name, value) {
            tracing::debug!(property = name, ?value, "réglage appliqué");
        }
    }
}

/// Fixe une propriété si elle existe, est modifiable et accepte la valeur.
/// Ne panique jamais : renvoie faux si la propriété est absente ou incompatible.
pub fn set_if_exists(element: &impl IsA<gst::Object>, name: &str, value: &PropValue) -> bool {
    let obj = element.upcast_ref::<gst::Object>();
    let Some(pspec) = obj.find_property(name) else {
        return false;
    };
    if !pspec.flags().contains(glib::ParamFlags::WRITABLE) {
        return false;
    }
    let ty = pspec.value_type();
    match value {
        PropValue::Bool(b) if ty == glib::Type::BOOL => obj.set_property(name, *b),
        PropValue::Uint(n) => {
            let n = *n;
            if ty == glib::Type::U32 {
                obj.set_property(name, n.min(u64::from(u32::MAX)) as u32)
            } else if ty == glib::Type::I32 {
                obj.set_property(name, n.min(i32::MAX as u64) as i32)
            } else if ty == glib::Type::U64 {
                obj.set_property(name, n)
            } else if ty == glib::Type::I64 {
                obj.set_property(name, n.min(i64::MAX as u64) as i64)
            } else if ty == glib::Type::U_LONG {
                obj.set_property(name, n.min(u64::from(u32::MAX)) as std::ffi::c_ulong)
            } else if ty == glib::Type::I_LONG {
                obj.set_property(name, n.min(i32::MAX as u64) as std::ffi::c_long)
            } else {
                return false;
            }
        }
        PropValue::Nick(nicks) => {
            let Some(nick) = nicks.iter().find(|n| accepts_nick(&pspec, n)) else {
                return false;
            };
            if ty == glib::Type::STRING {
                obj.set_property(name, *nick);
            } else {
                obj.set_property_from_str(name, nick);
            }
        }
        _ => return false,
    }
    true
}

fn accepts_nick(pspec: &glib::ParamSpec, nick: &str) -> bool {
    if let Some(e) = pspec.downcast_ref::<glib::ParamSpecEnum>() {
        return e.enum_class().value_by_nick(nick).is_some();
    }
    if let Some(f) = pspec.downcast_ref::<glib::ParamSpecFlags>() {
        return f.flags_class().value_by_nick(nick).is_some();
    }
    pspec.value_type() == glib::Type::STRING
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_order_per_platform() {
        assert_eq!(
            auto_candidates("macos"),
            ["vtenc_h264_hw", "vtenc_h264", "openh264enc"]
        );
        assert_eq!(
            auto_candidates("windows"),
            [
                "nvh264enc",
                "qsvh264enc",
                "amfh264enc",
                "mfh264enc",
                "openh264enc"
            ]
        );
        assert_eq!(
            auto_candidates("linux"),
            ["nvh264enc", "vah264enc", "vaapih264enc", "openh264enc"]
        );
        assert_eq!(auto_candidates("freebsd"), ["openh264enc"]);
    }

    #[test]
    fn x264_never_auto() {
        for os in ["macos", "windows", "linux", "other"] {
            assert!(!auto_candidates(os).contains(&GPL_ENCODER));
            assert_eq!(listed_candidates(os).last(), Some(&GPL_ENCODER));
        }
        // Même si x264enc est le seul encodeur présent, l'automatique ne le prend pas.
        let only_x264 = |n: &str| n == "x264enc";
        assert!(select_with(&EncoderChoice::Auto, "linux", only_x264).is_err());
        assert_eq!(
            select_with(&EncoderChoice::Named("x264enc".into()), "linux", only_x264).unwrap(),
            "x264enc"
        );
    }

    #[test]
    fn auto_prefers_hardware() {
        let avail = |n: &str| matches!(n, "vah264enc" | "openh264enc");
        assert_eq!(
            select_with(&EncoderChoice::Auto, "linux", avail).unwrap(),
            "vah264enc"
        );
        let soft = |n: &str| n == "openh264enc";
        assert_eq!(
            select_with(&EncoderChoice::Auto, "windows", soft).unwrap(),
            "openh264enc"
        );
        assert!(select_with(&EncoderChoice::Named("nvh264enc".into()), "linux", soft).is_err());
    }

    #[test]
    fn parse_choice() {
        assert_eq!(
            "auto".parse::<EncoderChoice>().unwrap(),
            EncoderChoice::Auto
        );
        assert_eq!(
            "openh264enc".parse::<EncoderChoice>().unwrap(),
            EncoderChoice::Named("openh264enc".into())
        );
        assert!("".parse::<EncoderChoice>().is_err());
    }

    #[test]
    fn aac_order() {
        assert_eq!(select_aac_with(|_| true).unwrap(), "avenc_aac");
        assert_eq!(select_aac_with(|n| n != "avenc_aac").unwrap(), "voaacenc");
        assert_eq!(select_aac_with(|n| n == "fdkaacenc").unwrap(), "fdkaacenc");
        assert!(select_aac_with(|_| false).is_err());
    }

    #[test]
    fn tuning_units() {
        let t = video_tuning("openh264enc", 6000, 30);
        assert!(t.contains(&("bitrate", PropValue::Uint(6_000_000))));
        assert!(t.contains(&("gop-size", PropValue::Uint(60))));
        let t = video_tuning("nvh264enc", 4500, 25);
        assert!(t.contains(&("bitrate", PropValue::Uint(4500))));
        assert!(t.contains(&("gop-size", PropValue::Uint(50))));
    }

    #[test]
    fn tuning_applies_only_existing_properties() {
        gst::init().unwrap();
        let enc = gst::ElementFactory::make("openh264enc").build().unwrap();
        apply(&enc, &video_tuning("openh264enc", 3000, 30));
        assert_eq!(enc.property::<u32>("bitrate"), 3_000_000);
        assert_eq!(enc.property::<u32>("gop-size"), 60);
        // Propriété inexistante ou nick invalide : ignorés sans paniquer.
        assert!(!set_if_exists(&enc, "n-existe-pas", &PropValue::Bool(true)));
        assert!(!set_if_exists(
            &enc,
            "rate-control",
            &PropValue::Nick(&["mode-inconnu"])
        ));
    }

    #[test]
    fn probe_software_fallback() {
        assert!(probe("openh264enc"));
        assert!(!probe("encodeur-qui-n-existe-pas"));
        assert!(!is_hardware("openh264enc"));
        assert!(is_hardware("nvh264enc"));
    }
}
