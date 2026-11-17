//! Moteur : état partagé, boucle de rendu et gestion des sorties.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use studio_core::{Change, Compositor, Studio};
use suite_protocol::ServerMessage;
use tokio::sync::broadcast;

use crate::outputs::{FfmpegOutput, Output, OutputKind};

struct ActiveOutput {
    output: Box<dyn Output>,
    started: Instant,
    frames: u64,
}

pub struct Engine {
    pub studio: Mutex<Studio>,
    outputs: Mutex<HashMap<OutputKind, ActiveOutput>>,
    compositor: Mutex<Compositor>,
    events: broadcast::Sender<ServerMessage>,
    frame: Mutex<u64>,
    pub recordings_dir: std::path::PathBuf,
}

impl Engine {
    pub fn new(studio: Studio, recordings_dir: std::path::PathBuf) -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        Arc::new(Self {
            studio: Mutex::new(studio),
            outputs: Mutex::new(HashMap::new()),
            compositor: Mutex::new(Compositor::new()),
            events,
            frame: Mutex::new(0),
            recordings_dir,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerMessage> {
        self.events.subscribe()
    }

    pub fn emit(&self, event_type: &str, data: Value) {
        let _ = self.events.send(ServerMessage::Event {
            app: "studio".into(),
            event_type: event_type.to_string(),
            data,
        });
    }

    pub fn emit_change(&self, change: &Change) {
        let v = serde_json::to_value(change).expect("Change sérialisable");
        self.emit(
            v["type"].as_str().unwrap_or("Unknown"),
            v.get("data").cloned().unwrap_or(Value::Null),
        );
    }

    pub fn frame_count(&self) -> u64 {
        *self.frame.lock().unwrap()
    }

    /// Image de la scène demandée (programme par défaut), encodée en PNG ou JPEG.
    pub fn screenshot(&self, scene_key: Option<&str>, jpeg: bool) -> Result<Vec<u8>, String> {
        let (canvas, scene) = {
            let studio = self.studio.lock().unwrap();
            let scene = match scene_key {
                Some(k) => studio.find_scene(k).map_err(|e| e.to_string())?.clone(),
                None => studio.program().clone(),
            };
            (studio.canvas, scene)
        };
        let img = self
            .compositor
            .lock()
            .unwrap()
            .render_image(canvas, &scene, self.frame_count());
        let mut out = std::io::Cursor::new(Vec::new());
        let fmt = if jpeg {
            image::ImageFormat::Jpeg
        } else {
            image::ImageFormat::Png
        };
        image::DynamicImage::ImageRgba8(img)
            .to_rgb8()
            .write_to(&mut out, fmt)
            .map_err(|e| e.to_string())?;
        Ok(out.into_inner())
    }

    pub fn start_output(&self, kind: OutputKind, target: Option<String>) -> Result<Value, String> {
        let mut outputs = self.outputs.lock().unwrap();
        if outputs.contains_key(&kind) {
            return Err("output_already_active".into());
        }
        let target = match (kind, target) {
            (_, Some(t)) => t,
            (OutputKind::Record, None) => {
                std::fs::create_dir_all(&self.recordings_dir).map_err(|e| e.to_string())?;
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                self.recordings_dir
                    .join(format!("enregistrement-{ts}.mp4"))
                    .to_string_lossy()
                    .into_owned()
            }
            (OutputKind::Stream, None) => return Err("missing_url".into()),
        };
        let canvas = self.studio.lock().unwrap().canvas;
        let output =
            FfmpegOutput::start(kind, canvas, &target).map_err(|e| format!("ffmpeg: {e}"))?;
        outputs.insert(
            kind,
            ActiveOutput {
                output: Box::new(output),
                started: Instant::now(),
                frames: 0,
            },
        );
        drop(outputs);
        self.emit("OutputStarted", json!({ "output": kind, "target": target }));
        Ok(json!({ "output": kind, "target": target }))
    }

    pub fn stop_output(&self, kind: OutputKind) -> Result<Value, String> {
        let active = self
            .outputs
            .lock()
            .unwrap()
            .remove(&kind)
            .ok_or("output_not_active")?;
        let target = active.output.target().to_string();
        let frames = active.frames;
        let duration = active.started.elapsed().as_secs_f64();
        active.output.stop();
        let data =
            json!({ "output": kind, "target": target, "frames": frames, "duration_s": duration });
        self.emit("OutputStopped", data.clone());
        Ok(data)
    }

    pub fn output_status(&self) -> Value {
        let outputs = self.outputs.lock().unwrap();
        let status = |k: OutputKind| match outputs.get(&k) {
            Some(a) => json!({
                "active": true,
                "target": a.output.target(),
                "frames": a.frames,
                "duration_s": a.started.elapsed().as_secs_f64(),
            }),
            None => json!({ "active": false }),
        };
        json!({ "record": status(OutputKind::Record), "stream": status(OutputKind::Stream) })
    }

    /// Arrête proprement toutes les sorties actives (fichiers finalisés, direct coupé).
    pub fn stop_all_outputs(&self) {
        for kind in [OutputKind::Record, OutputKind::Stream] {
            if self.stop_output(kind).is_ok() {
                tracing::info!("sortie {kind:?} arrêtée");
            }
        }
    }

    pub fn has_active_outputs(&self) -> bool {
        !self.outputs.lock().unwrap().is_empty()
    }

    /// Boucle de rendu à la cadence du canvas, dans un thread dédié.
    pub fn spawn_render_loop(self: &Arc<Self>) {
        let engine = Arc::clone(self);
        std::thread::Builder::new()
            .name("render".into())
            .spawn(move || {
                let mut buf = Vec::new();
                let mut next = Instant::now();
                loop {
                    let (canvas, scene) = {
                        let s = engine.studio.lock().unwrap();
                        (s.canvas, s.program().clone())
                    };
                    let frame = {
                        let mut f = engine.frame.lock().unwrap();
                        *f += 1;
                        *f
                    };
                    let has_outputs = engine.has_active_outputs();
                    if has_outputs {
                        engine
                            .compositor
                            .lock()
                            .unwrap()
                            .render(canvas, &scene, frame, &mut buf);
                        let mut failed = Vec::new();
                        {
                            let mut outputs = engine.outputs.lock().unwrap();
                            for (kind, active) in outputs.iter_mut() {
                                match active.output.push_frame(&buf) {
                                    Ok(()) => active.frames += 1,
                                    Err(e) => failed.push((*kind, e.to_string())),
                                }
                            }
                        }
                        for (kind, error) in failed {
                            tracing::warn!(?kind, %error, "sortie en échec");
                            let _ = engine.stop_output(kind);
                            engine.emit("OutputFailed", json!({ "output": kind, "error": error }));
                        }
                    }
                    let period = Duration::from_secs_f64(1.0 / canvas.fps.max(1) as f64);
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else {
                        // En retard : on se recale plutôt que d'accumuler.
                        next = now;
                    }
                }
            })
            .expect("thread de rendu");
    }
}
