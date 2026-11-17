//! Moteur : état partagé, boucle de rendu et gestion des sorties.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use studio_core::{Change, Compositor, Studio};
use suite_protocol::ServerMessage;
use tokio::sync::broadcast;

use crate::outputs::{
    self, AudioSink, Backend, EncoderChoice, Output, OutputKind, OutputSettings, RecordFormat,
    AUDIO_CHANNELS, DEFAULT_RECORD_BITRATE_KBPS, DEFAULT_STREAM_BITRATE_KBPS,
};

/// Réglages des sorties fixés au lancement de `studiod`.
#[derive(Debug, Clone)]
pub struct OutputConfig {
    pub backend: Backend,
    pub encoder: EncoderChoice,
    pub record_bitrate_kbps: u32,
    pub stream_bitrate_kbps: u32,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            backend: if cfg!(feature = "gstreamer") {
                Backend::Gstreamer
            } else {
                Backend::Ffmpeg
            },
            encoder: EncoderChoice::Auto,
            record_bitrate_kbps: DEFAULT_RECORD_BITRATE_KBPS,
            stream_bitrate_kbps: DEFAULT_STREAM_BITRATE_KBPS,
        }
    }
}

/// Options d'une requête de démarrage de sortie (champs optionnels de l'API).
#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    pub target: Option<String>,
    pub format: Option<RecordFormat>,
    pub fragmented: bool,
    pub bitrate_kbps: Option<u32>,
}

struct ActiveOutput {
    output: Box<dyn Output>,
    /// Entrée audio : le moteur y pousse du silence calé sur les images.
    audio: Option<Arc<dyn AudioSink>>,
    started: Instant,
    /// Images envoyées à la sortie (y compris celles qu'elle a dû perdre).
    frames: u64,
    format: Option<RecordFormat>,
    bitrate_kbps: u32,
}

impl ActiveOutput {
    fn describe(&self, kind: OutputKind, backend: Backend) -> Value {
        let mut v = json!({
            "output": kind,
            "target": self.output.target(),
            "backend": backend,
            "encoder": self.output.encoder(),
            "bitrate_kbps": self.bitrate_kbps,
        });
        if let Some(f) = self.format {
            v["format"] = json!(f);
        }
        v
    }
}

pub struct Engine {
    pub studio: Mutex<Studio>,
    outputs: Mutex<HashMap<OutputKind, ActiveOutput>>,
    compositor: Mutex<Compositor>,
    events: broadcast::Sender<ServerMessage>,
    frame: Mutex<u64>,
    pub recordings_dir: std::path::PathBuf,
    pub output_config: OutputConfig,
}

impl Engine {
    pub fn new(
        studio: Studio,
        recordings_dir: std::path::PathBuf,
        output_config: OutputConfig,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        Arc::new(Self {
            studio: Mutex::new(studio),
            outputs: Mutex::new(HashMap::new()),
            compositor: Mutex::new(Compositor::new()),
            events,
            frame: Mutex::new(0),
            recordings_dir,
            output_config,
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

    /// Encodeurs H.264 connus et celui qui serait utilisé (requête `GetEncoders`).
    pub fn encoders(&self) -> Value {
        let cfg = &self.output_config;
        #[cfg(feature = "gstreamer")]
        let list = match cfg.backend {
            Backend::Gstreamer => json!(crate::encoders::list(&cfg.encoder)),
            Backend::Ffmpeg => json!([]),
        };
        #[cfg(not(feature = "gstreamer"))]
        let list = json!([]);
        let requested = match &cfg.encoder {
            EncoderChoice::Auto => "auto".to_string(),
            EncoderChoice::Named(n) => n.clone(),
        };
        json!({ "backend": cfg.backend, "requested": requested, "encoders": list })
    }

    pub fn start_output(&self, kind: OutputKind, opts: StartOptions) -> Result<Value, String> {
        let mut outputs = self.outputs.lock().unwrap();
        if outputs.contains_key(&kind) {
            return Err("output_already_active".into());
        }
        let bitrate_kbps = opts.bitrate_kbps.unwrap_or(match kind {
            OutputKind::Record => self.output_config.record_bitrate_kbps,
            OutputKind::Stream => self.output_config.stream_bitrate_kbps,
        });
        if !(100..=100_000).contains(&bitrate_kbps) {
            return Err("invalid_bitrate".into());
        }
        let (target, format) = match (kind, opts.target) {
            (OutputKind::Record, Some(t)) => {
                let format = opts.format.unwrap_or_else(|| RecordFormat::from_path(&t));
                (t, Some(format))
            }
            (OutputKind::Record, None) => {
                let format = opts.format.unwrap_or_default();
                std::fs::create_dir_all(&self.recordings_dir).map_err(|e| e.to_string())?;
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let path = self
                    .recordings_dir
                    .join(format!("enregistrement-{ts}.{}", format.extension()))
                    .to_string_lossy()
                    .into_owned();
                (path, Some(format))
            }
            (OutputKind::Stream, Some(t)) => {
                if outputs::StreamProtocol::from_url(&t).is_none() {
                    return Err("unsupported_url".into());
                }
                (t, None)
            }
            (OutputKind::Stream, None) => return Err("missing_url".into()),
        };
        let canvas = self.studio.lock().unwrap().canvas;
        let settings = OutputSettings {
            kind,
            canvas,
            target,
            format: format.unwrap_or_default(),
            fragmented: opts.fragmented,
            bitrate_kbps,
            encoder: self.output_config.encoder.clone(),
        };
        let output = outputs::start(self.output_config.backend, &settings)?;
        let active = ActiveOutput {
            audio: output.audio_sink(),
            output,
            started: Instant::now(),
            frames: 0,
            format,
            bitrate_kbps,
        };
        let data = active.describe(kind, self.output_config.backend);
        outputs.insert(kind, active);
        drop(outputs);
        self.emit("OutputStarted", data.clone());
        Ok(data)
    }

    pub fn stop_output(&self, kind: OutputKind) -> Result<Value, String> {
        let active = self
            .outputs
            .lock()
            .unwrap()
            .remove(&kind)
            .ok_or("output_not_active")?;
        let mut data = active.describe(kind, self.output_config.backend);
        data["frames"] = json!(active.frames);
        data["dropped_frames"] = json!(active.output.dropped_frames());
        data["duration_s"] = json!(active.started.elapsed().as_secs_f64());
        let finalized = active.output.stop();
        data["finalized"] = json!(finalized.is_ok());
        if let Err(e) = finalized {
            tracing::warn!(?kind, error = %e, "arrêt incomplet de la sortie");
            data["warning"] = json!(e);
        }
        self.emit("OutputStopped", data.clone());
        Ok(data)
    }

    pub fn output_status(&self) -> Value {
        let outputs = self.outputs.lock().unwrap();
        let backend = self.output_config.backend;
        let status = |k: OutputKind| match outputs.get(&k) {
            Some(a) => {
                let mut v = a.describe(k, backend);
                v["active"] = json!(true);
                v["frames"] = json!(a.frames);
                v["dropped_frames"] = json!(a.output.dropped_frames());
                v["duration_s"] = json!(a.started.elapsed().as_secs_f64());
                v
            }
            None => json!({ "active": false }),
        };
        json!({
            "backend": backend,
            "record": status(OutputKind::Record),
            "stream": status(OutputKind::Stream),
        })
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
                let mut silence: Vec<i16> = Vec::new();
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
                                let n = active.frames;
                                if let Err(e) = active.output.push_frame(&buf) {
                                    failed.push((*kind, e.to_string()));
                                    continue;
                                }
                                active.frames += 1;
                                // Silence calé sur l'horodatage de cette image ; le mixeur
                                // audio poussera ici le vrai son.
                                if let Some(audio) = &active.audio {
                                    let samples = outputs::audio_samples_for_frame(n, canvas.fps)
                                        * AUDIO_CHANNELS as usize;
                                    silence.resize(samples, 0);
                                    if let Err(e) = audio.push_audio(&silence[..samples]) {
                                        failed.push((*kind, e.to_string()));
                                    }
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
