//! Traitement des requêtes de l'API WebSocket.

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use studio_core::{Canvas, Change, SourceKind, SourcePatch, StudioError, Transform};

use crate::engine::Engine;
use crate::outputs::OutputKind;

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
    },
    StopRecording,
    StartStream {
        url: String,
    },
    StopStream,
    GetOutputStatus,
}

pub const CAPABILITIES: &[&str] = &[
    "core",
    "studio.scenes",
    "studio.sources",
    "studio.outputs",
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
                "app": { "id": "studio", "name": "Studio", "version": env!("CARGO_PKG_VERSION") },
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
            if engine.has_active_outputs() {
                return Err((
                    "outputs_active".into(),
                    "arrête les sorties avant de changer le canvas".into(),
                ));
            }
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
            studio().canvas = canvas;
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
        StartRecording { path } => {
            return engine
                .start_output(OutputKind::Record, path)
                .map_err(output_err)
        }
        StopRecording => return engine.stop_output(OutputKind::Record).map_err(output_err),
        StartStream { url } => {
            return engine
                .start_output(OutputKind::Stream, Some(url))
                .map_err(output_err)
        }
        StopStream => return engine.stop_output(OutputKind::Stream).map_err(output_err),
        GetOutputStatus => return Ok(engine.output_status()),
    };
    change.map(|c| apply(engine, vec![c])).map_err(studio_err)
}
