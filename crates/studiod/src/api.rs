//! Traitement des requêtes de l'API WebSocket.

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use studio_core::{Canvas, Change, SourceKind, SourcePatch, StudioError, Transform};

use crate::engine::{Engine, StartOptions};
use crate::outputs::{OutputKind, RecordFormat};

/// Liste des requêtes, typées. Le nom de variante est le champ `type` du message.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Request {
    // Domaine `core`, commun à toutes les apps de la suite.
    GetAppInfo,
    GetRequestList,
    Ping,
    // Domaine `studio.*`.
    GetStudio,
    GetSceneList,
    GetScene {
        scene: String,
    },
    CreateScene {
        name: String,
    },
    RemoveScene {
        scene: String,
    },
    RenameScene {
        scene: String,
        name: String,
    },
    SetProgramScene {
        scene: String,
    },
    SetPreviewScene {
        scene: String,
    },
    Cut,
    AddSource {
        scene: String,
        name: String,
        #[serde(flatten)]
        kind: SourceKind,
        #[serde(default)]
        transform: Option<Transform>,
    },
    RemoveSource {
        scene: String,
        source: String,
    },
    UpdateSource {
        scene: String,
        source: String,
        #[serde(flatten)]
        patch: SourcePatch,
    },
    MoveSource {
        scene: String,
        source: String,
        index: usize,
    },
    SetCanvas {
        width: u32,
        height: u32,
        fps: u32,
    },
    GetScreenshot {
        #[serde(default)]
        scene: Option<String>,
        #[serde(default)]
        format: Option<String>,
    },
    StartRecording {
        #[serde(default)]
        path: Option<String>,
        /// `mp4` (défaut) ou `mkv` ; déduit de l'extension de `path` si absent.
        #[serde(default)]
        format: Option<RecordFormat>,
        /// MP4 fragmenté, lisible même après un arrêt brutal.
        #[serde(default)]
        fragmented: bool,
        #[serde(default)]
        bitrate_kbps: Option<u32>,
    },
    StopRecording,
    StartStream {
        url: String,
        #[serde(default)]
        bitrate_kbps: Option<u32>,
    },
    StopStream,
    GetOutputStatus,
    GetEncoders,
}

pub const CAPABILITIES: &[&str] = &[
    "core",
    "studio.scenes",
    "studio.sources",
    "studio.outputs",
    "studio.encoders",
    "studio.preview",
];

pub const REQUESTS: &[&str] = &[
    "GetAppInfo",
    "GetRequestList",
    "Ping",
    "GetStudio",
    "GetSceneList",
    "GetScene",
    "CreateScene",
    "RemoveScene",
    "RenameScene",
    "SetProgramScene",
    "SetPreviewScene",
    "Cut",
    "AddSource",
    "RemoveSource",
    "UpdateSource",
    "MoveSource",
    "SetCanvas",
    "GetScreenshot",
    "StartRecording",
    "StopRecording",
    "StartStream",
    "StopStream",
    "GetOutputStatus",
    "GetEncoders",
];

/// Erreur renvoyée au client : (code, message).
pub type ApiError = (String, String);

fn studio_err(e: StudioError) -> ApiError {
    let code = match e {
        StudioError::SceneNotFound(_) => "scene_not_found",
        StudioError::SourceNotFound(_) => "source_not_found",
        StudioError::SceneNameTaken(_) => "scene_name_taken",
        StudioError::SceneOnAir => "scene_on_air",
        StudioError::LastScene => "last_scene",
    };
    (code.to_string(), e.to_string())
}

fn output_err(code: String) -> ApiError {
    let message = match code.as_str() {
        "output_already_active" => "cette sortie est déjà active".to_string(),
        "output_not_active" => "cette sortie n'est pas active".to_string(),
        "missing_url" => "URL de diffusion manquante".to_string(),
        "unsupported_url" => {
            "URL non prise en charge : rtmp://, rtmps:// ou srt:// attendu".to_string()
        }
        "invalid_bitrate" => "débit vidéo invalide : entre 100 et 100000 kbit/s".to_string(),
        "shutting_down" => "studiod s'arrête : aucune sortie ne peut démarrer".to_string(),
        other => return ("output_error".into(), other.to_string()),
    };
    (code, message)
}

/// Décode puis exécute une requête. Les changements d'état sont diffusés en événements.
pub fn handle(engine: &Engine, request_type: &str, data: Value) -> Result<Value, ApiError> {
    if !REQUESTS.contains(&request_type) {
        return Err((
            "unknown_request".into(),
            format!("requête inconnue : {request_type}"),
        ));
    }
    let mut envelope = json!({ "type": request_type });
    if !data.is_null() {
        envelope["data"] = data;
    }
    let req: Request = serde_json::from_value(envelope).map_err(|e| {
        (
            "invalid_data".to_string(),
            format!("données invalides : {e}"),
        )
    })?;
    execute(engine, req)
}

fn apply(engine: &Engine, changes: Vec<Change>) -> Value {
    for c in &changes {
        engine.emit_change(c);
    }
    match changes.as_slice() {
        [one] => serde_json::to_value(one).unwrap()["data"].clone(),
        _ => json!({}),
    }
}

fn execute(engine: &Engine, req: Request) -> Result<Value, ApiError> {
    use Request::*;
    let studio = || engine.studio.lock().unwrap();
    let change = match req {
        GetAppInfo => {
            return Ok(json!({
                "app": { "id": "studio", "name": "Maya Studio", "version": env!("CARGO_PKG_VERSION") },
                "protocol": suite_protocol::PROTOCOL_VERSION,
                "capabilities": CAPABILITIES,
            }))
        }
        GetRequestList => return Ok(json!({ "requests": REQUESTS })),
        Ping => return Ok(json!({ "pong": true })),
        GetStudio => return Ok(serde_json::to_value(&*studio()).unwrap()),
        GetSceneList => {
            let s = studio();
            let scenes: Vec<Value> = s
                .scenes
                .iter()
                .map(|sc| json!({ "id": sc.id, "name": sc.name, "sources": sc.sources.len() }))
                .collect();
            return Ok(
                json!({ "scenes": scenes, "program_scene": s.program_scene, "preview_scene": s.preview_scene }),
            );
        }
        GetScene { scene } => {
            return studio()
                .find_scene(&scene)
                .map(|s| serde_json::to_value(s).unwrap())
                .map_err(studio_err)
        }
        CreateScene { name } => studio().create_scene(&name),
        RemoveScene { scene } => studio().remove_scene(&scene),
        RenameScene { scene, name } => studio().rename_scene(&scene, &name),
        SetProgramScene { scene } => studio().set_program_scene(&scene),
        SetPreviewScene { scene } => studio().set_preview_scene(&scene),
        Cut => {
            let changes = studio().cut();
            return Ok(apply(engine, changes));
        }
        AddSource {
            scene,
            name,
            kind,
            transform,
        } => studio().add_source(&scene, &name, kind, transform),
        RemoveSource { scene, source } => studio().remove_source(&scene, &source),
        UpdateSource {
            scene,
            source,
            patch,
        } => studio().update_source(&scene, &source, patch),
        MoveSource {
            scene,
            source,
            index,
        } => studio().move_source(&scene, &source, index),
        SetCanvas { width, height, fps } => {
            if width == 0
                || height == 0
                || width % 2 == 1
                || height % 2 == 1
                || fps == 0
                || fps > 120
            {
                return Err((
                    "invalid_data".into(),
                    "largeur et hauteur paires, fps entre 1 et 120".into(),
                ));
            }
            let canvas = Canvas { width, height, fps };
            // Vérification des sorties et écriture atomiques (voir Engine::set_canvas).
            engine.set_canvas(canvas).map_err(|code| {
                (
                    code,
                    "arrête les sorties avant de changer le canvas".to_string(),
                )
            })?;
            engine.emit("CanvasChanged", json!(canvas));
            return Ok(json!(canvas));
        }
        GetScreenshot { scene, format } => {
            let jpeg = matches!(format.as_deref(), Some("jpeg") | Some("jpg"));
            let bytes = engine
                .screenshot(scene.as_deref(), jpeg)
                .map_err(|e| ("screenshot_failed".to_string(), e))?;
            let mime = if jpeg { "image/jpeg" } else { "image/png" };
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            return Ok(json!({ "image": format!("data:{mime};base64,{b64}") }));
        }
        StartRecording {
            path,
            format,
            fragmented,
            bitrate_kbps,
        } => {
            let opts = StartOptions {
                target: path,
                format,
                fragmented,
                bitrate_kbps,
            };
            return engine
                .start_output(OutputKind::Record, opts)
                .map_err(output_err);
        }
        StopRecording => return engine.stop_output(OutputKind::Record).map_err(output_err),
        StartStream { url, bitrate_kbps } => {
            let opts = StartOptions {
                target: Some(url),
                bitrate_kbps,
                ..Default::default()
            };
            return engine
                .start_output(OutputKind::Stream, opts)
                .map_err(output_err);
        }
        StopStream => return engine.stop_output(OutputKind::Stream).map_err(output_err),
        GetOutputStatus => return Ok(engine.output_status()),
        GetEncoders => return Ok(engine.encoders()),
    };
    change.map(|c| apply(engine, vec![c])).map_err(studio_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::OutputConfig;
    use studio_core::Studio;

    fn engine() -> std::sync::Arc<Engine> {
        let canvas = Canvas {
            width: 64,
            height: 36,
            fps: 30,
        };
        Engine::new(
            Studio::new(canvas),
            std::env::temp_dir(),
            OutputConfig::default(),
        )
    }

    #[test]
    fn output_requests_parse_optional_fields() {
        let req: Request = serde_json::from_value(json!({
            "type": "StartRecording",
            "data": { "format": "mkv", "fragmented": true, "bitrate_kbps": 8000 }
        }))
        .unwrap();
        assert!(matches!(
            req,
            Request::StartRecording {
                path: None,
                format: Some(RecordFormat::Mkv),
                fragmented: true,
                bitrate_kbps: Some(8000)
            }
        ));
        // Anciennes formes, toujours acceptées (rétrocompatibilité).
        let req: Request =
            serde_json::from_value(json!({ "type": "StartRecording", "data": {} })).unwrap();
        assert!(matches!(
            req,
            Request::StartRecording {
                format: None,
                fragmented: false,
                ..
            }
        ));
        let req: Request = serde_json::from_value(
            json!({ "type": "StartStream", "data": { "url": "srt://h:1" } }),
        )
        .unwrap();
        assert!(matches!(
            req,
            Request::StartStream {
                bitrate_kbps: None,
                ..
            }
        ));
        assert!(serde_json::from_value::<Request>(
            json!({ "type": "StartRecording", "data": { "format": "avi" } })
        )
        .is_err());
    }

    #[test]
    fn get_encoders_listed_and_answered() {
        assert!(REQUESTS.contains(&"GetEncoders"));
        assert!(CAPABILITIES.contains(&"studio.encoders"));
        let v = handle(&engine(), "GetEncoders", Value::Null).unwrap();
        assert_eq!(v["requested"], "auto");
        let list = v["encoders"].as_array().unwrap();
        if cfg!(feature = "gstreamer") {
            assert_eq!(v["backend"], "gstreamer");
            let selected: Vec<_> = list.iter().filter(|e| e["selected"] == true).collect();
            assert!(selected.len() <= 1);
            let x264 = list.iter().find(|e| e["name"] == "x264enc").unwrap();
            assert_eq!(x264["selected"], false);
            assert_eq!(x264["gpl"], true);
        }
    }

    #[test]
    fn stream_url_and_bitrate_validated() {
        let e = engine();
        let err = handle(&e, "StartStream", json!({ "url": "udp://x" })).unwrap_err();
        assert_eq!(err.0, "unsupported_url");
        let err = handle(
            &e,
            "StartStream",
            json!({ "url": "srt://h:1", "bitrate_kbps": 5 }),
        )
        .unwrap_err();
        assert_eq!(err.0, "invalid_bitrate");
    }
}
