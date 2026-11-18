//! Moteur : état partagé, boucle de rendu et gestion des sorties.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use studio_core::{Canvas, Change, Compositor, SourceKind, Studio};
use suite_protocol::ServerMessage;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::media::MediaStore;
use crate::mixer::{self, LevelMeter, MixBus, BLOCK_FRAMES, BLOCK_MS, BLOCK_SAMPLES};
use crate::outputs::{
    self, AudioSink, Backend, EncoderChoice, Output, OutputKind, OutputSettings, RecordFormat,
    AUDIO_CHANNELS, DEFAULT_RECORD_BITRATE_KBPS, DEFAULT_STREAM_BITRATE_KBPS,
};

/// Période des événements `InputLevels`, en blocs de mixage (10 × 10 ms).
const LEVELS_EVERY_BLOCKS: u64 = 10;

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

/// Options de `GetScreenshot`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScreenshotOptions {
    pub jpeg: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Qualité JPEG de 1 à 100 (80 par défaut).
    pub quality: Option<u8>,
}

impl ScreenshotOptions {
    /// Taille de l'image rendue : celle demandée, sans dépasser le canvas. Avec une
    /// seule dimension, l'autre suit le rapport largeur / hauteur du canvas.
    pub fn size(&self, canvas: Canvas) -> Option<(u32, u32)> {
        let (cw, ch) = (canvas.width.max(1), canvas.height.max(1));
        let (w, h) = match (self.width, self.height) {
            (None, None) => return None,
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, (u64::from(w) * u64::from(ch) / u64::from(cw)) as u32),
            (None, Some(h)) => ((u64::from(h) * u64::from(cw) / u64::from(ch)) as u32, h),
        };
        Some((w.clamp(1, cw), h.clamp(1, ch)))
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
    /// Canvas avec lequel la sortie a été configurée (taille des images, cadence).
    canvas: Canvas,
    /// Entrée audio : le mixeur y pousse le son, calé sur les images.
    audio: Option<Arc<dyn AudioSink>>,
    /// Échantillons (par canal) déjà poussés dans `audio` : position de la piste audio.
    audio_pos: u64,
    started: Instant,
    /// Début de la pause en cours (enregistrement seulement).
    paused_since: Option<Instant>,
    /// Durée cumulée des pauses terminées.
    paused_total: Duration,
    /// Images envoyées à la sortie (y compris celles qu'elle a dû perdre).
    frames: u64,
    format: Option<RecordFormat>,
    bitrate_kbps: u32,
}

impl ActiveOutput {
    /// Durée enregistrée ou diffusée, pauses exclues.
    fn duration(&self) -> Duration {
        let paused = self.paused_total + self.paused_since.map_or(Duration::ZERO, |p| p.elapsed());
        self.started.elapsed().saturating_sub(paused)
    }

    /// Champs d'état communs à `GetOutputStatus` et `OutputStopped`.
    fn counters(&self, v: &mut Value) {
        v["frames"] = json!(self.frames);
        v["dropped_frames"] = json!(self.output.dropped_frames());
        v["duration_s"] = json!(self.duration().as_secs_f64());
        v["paused"] = json!(self.paused_since.is_some());
        // Taille du fichier écrit (enregistrement) : permet de vérifier qu'il grossit.
        if self.format.is_some() {
            if let Ok(m) = std::fs::metadata(self.output.target()) {
                v["bytes"] = json!(m.len());
            }
        }
    }

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

/// Sorties actives et sorties en cours de démarrage, sous un même verrou.
#[derive(Default)]
struct Outputs {
    active: HashMap<OutputKind, ActiveOutput>,
    /// Emplacements réservés pendant la construction d'un pipeline, faite hors verrou
    /// pour ne pas bloquer la boucle de rendu ni les autres sorties.
    starting: HashSet<OutputKind>,
}

impl Outputs {
    fn is_idle(&self) -> bool {
        self.active.is_empty() && self.starting.is_empty()
    }
}

/// Libère l'emplacement réservé par `start_output`, même en cas d'erreur ou de panique.
struct StartingSlot<'a> {
    outputs: &'a Mutex<Outputs>,
    kind: OutputKind,
}

impl Drop for StartingSlot<'_> {
    fn drop(&mut self) {
        let mut outputs = self.outputs.lock().unwrap_or_else(|e| e.into_inner());
        outputs.starting.remove(&self.kind);
    }
}

/// Retient l'état annoncé par un événement de capture.
#[cfg_attr(not(feature = "gstreamer"), allow(dead_code))]
fn record_capture_state(states: &Mutex<HashMap<Uuid, Value>>, event_type: &str, data: &Value) {
    let state = match event_type {
        "SourceStarted" => json!({ "state": "running" }),
        "SourceFailed" => json!({
            "state": "failed",
            "error": data["error"],
            "retry_in_ms": data["retry_in_ms"],
        }),
        _ => return,
    };
    let Some(id) = data["source_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
    else {
        return;
    };
    states.lock().unwrap().insert(id, state);
}

/// Oublie l'état des captures retirées et de celles dont les réglages ont changé :
/// `CaptureManager::sync` relance ces dernières sous le même identifiant, et leur
/// nouveau pipeline ne dit rien avant la première image (`SourceStarted`) ou le
/// premier échec. Sans cela, `GetCaptureStatus` répondrait l'ancien état.
/// `synced` garde les réglages du dernier passage.
fn prune_capture_states(
    states: &mut HashMap<Uuid, Value>,
    synced: &mut HashMap<Uuid, SourceKind>,
    wanted: &[(Uuid, SourceKind)],
) {
    states.retain(|id, _| {
        let now = wanted.iter().find(|(w, _)| w == id).map(|(_, k)| k);
        match (now, synced.get(id)) {
            (Some(now), Some(before)) => now == before,
            (Some(_), None) => true,
            (None, _) => false,
        }
    });
    *synced = wanted.iter().cloned().collect();
}

pub struct Engine {
    pub studio: Mutex<Studio>,
    /// Ordre des verrous : `outputs` avant `studio`.
    outputs: Mutex<Outputs>,
    /// Arrêt de `studiod` demandé : plus aucune sortie ne peut démarrer.
    shutting_down: AtomicBool,
    compositor: Mutex<Compositor>,
    /// Dernières images et son des sources de capture.
    media: Arc<MediaStore>,
    #[cfg(feature = "gstreamer")]
    captures: crate::captures::CaptureManager,
    /// Dernier état connu de chaque capture (`SourceStarted` / `SourceFailed`), pour
    /// les clients qui se connectent après coup (`GetCaptureStatus`).
    capture_states: Arc<Mutex<HashMap<Uuid, Value>>>,
    /// Réglages des captures lors du dernier `sync_captures`.
    synced_capture_kinds: Mutex<HashMap<Uuid, SourceKind>>,
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
        let media = Arc::new(MediaStore::default());
        let capture_states: Arc<Mutex<HashMap<Uuid, Value>>> = Arc::default();
        #[cfg(feature = "gstreamer")]
        let captures = {
            let tx = events.clone();
            let states = Arc::clone(&capture_states);
            let emit: crate::captures::EmitFn = Arc::new(move |event_type: &str, data: Value| {
                record_capture_state(&states, event_type, &data);
                let _ = tx.send(ServerMessage::Event {
                    app: "studio".into(),
                    event_type: event_type.to_string(),
                    data,
                });
            });
            crate::captures::CaptureManager::new(Arc::clone(&media), emit)
        };
        Arc::new(Self {
            studio: Mutex::new(studio),
            outputs: Mutex::new(Outputs::default()),
            shutting_down: AtomicBool::new(false),
            compositor: Mutex::new(Compositor::new()),
            media,
            #[cfg(feature = "gstreamer")]
            captures,
            capture_states,
            synced_capture_kinds: Mutex::default(),
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

    /// Met les pipelines de capture en accord avec les sources des scènes. Appelé
    /// après chaque changement d'état et au lancement de la boucle de rendu.
    pub fn sync_captures(&self) {
        let wanted: Vec<(Uuid, SourceKind)> = self.studio.lock().unwrap().capture_list();
        // Les états des captures retirées ou relancées ne servent plus.
        prune_capture_states(
            &mut self.capture_states.lock().unwrap(),
            &mut self.synced_capture_kinds.lock().unwrap(),
            &wanted,
        );
        #[cfg(feature = "gstreamer")]
        self.captures.sync(&wanted);
        #[cfg(not(feature = "gstreamer"))]
        if !wanted.is_empty() {
            tracing::warn!("studiod compilé sans GStreamer : les sources de capture restent vides");
        }
    }

    /// Sources dont la capture tourne ou redémarre.
    pub fn running_captures(&self) -> Vec<Uuid> {
        #[cfg(feature = "gstreamer")]
        return self.captures.running();
        #[cfg(not(feature = "gstreamer"))]
        Vec::new()
    }

    /// État de chaque capture en cours (requête `GetCaptureStatus`) : `starting` tant
    /// qu'aucune image ni aucun son n'est arrivé, puis `running` ou `failed`.
    pub fn capture_status(&self) -> Value {
        let running: HashSet<Uuid> = self.running_captures().into_iter().collect();
        let (kinds, inputs): (Vec<(Uuid, Value)>, HashSet<Uuid>) = {
            let studio = self.studio.lock().unwrap();
            let kinds = studio
                .capture_list()
                .into_iter()
                .filter(|(id, _)| running.contains(id))
                .map(|(id, k)| (id, serde_json::to_value(&k).unwrap()["kind"].clone()))
                .collect();
            (kinds, studio.audio_inputs.iter().map(|i| i.id).collect())
        };
        let states = self.capture_states.lock().unwrap();
        let sources: Vec<Value> = kinds
            .into_iter()
            .map(|(id, kind)| {
                let mut v = states
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| json!({ "state": "starting" }));
                v["source_id"] = json!(id);
                v["kind"] = kind;
                // Entrée audio globale (`studio.mixer`) plutôt que source d'une scène.
                if inputs.contains(&id) {
                    v["input"] = json!(true);
                }
                v
            })
            .collect();
        json!({ "sources": sources })
    }

    /// Arrête toutes les captures (fermeture de `studiod`).
    pub fn shutdown_captures(&self) {
        #[cfg(feature = "gstreamer")]
        self.captures.stop_all();
    }

    /// Médias des sources de capture (images, son).
    pub fn media(&self) -> &Arc<MediaStore> {
        &self.media
    }

    /// Périphériques d'entrée (requête `GetInputDevices`).
    pub fn input_devices(&self) -> Value {
        #[cfg(feature = "gstreamer")]
        return json!(crate::devices::list());
        #[cfg(not(feature = "gstreamer"))]
        json!({ "video": [], "audio": [] })
    }

    /// Image de la scène demandée (programme par défaut), encodée en PNG ou JPEG,
    /// éventuellement réduite (miniatures de Nino).
    pub fn screenshot(
        &self,
        scene_key: Option<&str>,
        opts: ScreenshotOptions,
    ) -> Result<Vec<u8>, String> {
        let (canvas, scene) = {
            let studio = self.studio.lock().unwrap();
            let scene = match scene_key {
                Some(k) => studio.find_scene(k).map_err(|e| e.to_string())?.clone(),
                None => studio.program().clone(),
            };
            (studio.canvas, scene)
        };
        let img = self.compositor.lock().unwrap().render_image_with(
            canvas,
            &scene,
            self.frame_count(),
            &*self.media,
        );
        let mut img = image::DynamicImage::ImageRgba8(img).to_rgb8();
        if let Some((w, h)) = opts.size(canvas) {
            if (w, h) != (canvas.width, canvas.height) {
                img = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        if opts.jpeg {
            image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut out,
                opts.quality.unwrap_or(80),
            )
            .encode_image(&img)
            .map_err(|e| e.to_string())?;
        } else {
            img.write_to(&mut out, image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
        }
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
        let bitrate_kbps = opts.bitrate_kbps.unwrap_or(match kind {
            OutputKind::Record => self.output_config.record_bitrate_kbps,
            OutputKind::Stream => self.output_config.stream_bitrate_kbps,
        });
        if !(100..=100_000).contains(&bitrate_kbps) {
            return Err("invalid_bitrate".into());
        }
        if kind == OutputKind::Stream {
            match &opts.target {
                Some(t) if outputs::StreamProtocol::from_url(t).is_none() => {
                    return Err("unsupported_url".into())
                }
                None => return Err("missing_url".into()),
                Some(_) => {}
            }
        }
        // Réservation de l'emplacement et lecture du canvas sous le verrou des sorties :
        // `set_canvas` prend le même verrou, le canvas ne peut donc plus changer
        // tant que cette sortie démarre ou tourne.
        let (canvas, _slot) = {
            let mut outputs = self.outputs.lock().unwrap();
            if self.shutting_down.load(Ordering::SeqCst) {
                return Err("shutting_down".into());
            }
            if outputs.active.contains_key(&kind) || outputs.starting.contains(&kind) {
                return Err("output_already_active".into());
            }
            outputs.starting.insert(kind);
            let canvas = self.studio.lock().unwrap().canvas;
            (
                canvas,
                StartingSlot {
                    outputs: &self.outputs,
                    kind,
                },
            )
        };
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
            (OutputKind::Stream, t) => (t.expect("URL vérifiée plus haut"), None),
        };
        let settings = OutputSettings {
            kind,
            canvas,
            target,
            format: format.unwrap_or_default(),
            fragmented: opts.fragmented,
            bitrate_kbps,
            encoder: self.output_config.encoder.clone(),
        };
        // Construction et passage à PLAYING hors verrou : la boucle de rendu continue
        // d'alimenter les sorties déjà actives pendant ce temps.
        let output = outputs::start(self.output_config.backend, &settings)?;
        let active = ActiveOutput {
            audio: output.audio_sink(),
            audio_pos: 0,
            output,
            canvas,
            started: Instant::now(),
            paused_since: None,
            paused_total: Duration::ZERO,
            frames: 0,
            format,
            bitrate_kbps,
        };
        let data = active.describe(kind, self.output_config.backend);
        {
            let mut outputs = self.outputs.lock().unwrap();
            outputs.starting.remove(&kind);
            if self.shutting_down.load(Ordering::SeqCst) {
                drop(outputs);
                // Arrêt demandé pendant le démarrage : on finalise tout de suite.
                if let Err(e) = active.output.stop() {
                    tracing::warn!(?kind, error = %e, "arrêt incomplet de la sortie");
                }
                return Err("shutting_down".into());
            }
            outputs.active.insert(kind, active);
        }
        self.emit("OutputStarted", data.clone());
        Ok(data)
    }

    /// Change le canvas, uniquement si aucune sortie n'est active ni en démarrage.
    /// Vérification et écriture se font sous le verrou des sorties, sans fenêtre
    /// pendant laquelle une sortie pourrait démarrer avec l'ancien canvas.
    pub fn set_canvas(&self, canvas: Canvas) -> Result<(), String> {
        let outputs = self.outputs.lock().unwrap();
        if !outputs.is_idle() {
            return Err("outputs_active".into());
        }
        self.studio.lock().unwrap().canvas = canvas;
        Ok(())
    }

    /// Arrête toutes les sorties actives (arrêt de `studiod`) et refuse les suivantes.
    /// Les fichiers MP4 sont ainsi finalisés (EOS) avant la sortie du processus.
    pub fn shutdown_outputs(&self) -> Vec<Value> {
        let kinds: Vec<OutputKind> = {
            let outputs = self.outputs.lock().unwrap();
            self.shutting_down.store(true, Ordering::SeqCst);
            outputs.active.keys().copied().collect()
        };
        kinds
            .into_iter()
            .filter_map(|k| self.stop_output(k).ok())
            .collect()
    }

    pub fn stop_output(&self, kind: OutputKind) -> Result<Value, String> {
        let active = self
            .outputs
            .lock()
            .unwrap()
            .active
            .remove(&kind)
            .ok_or("output_not_active")?;
        let mut data = active.describe(kind, self.output_config.backend);
        active.counters(&mut data);
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
        let status = |k: OutputKind| match outputs.active.get(&k) {
            Some(a) => {
                let mut v = a.describe(k, backend);
                v["active"] = json!(true);
                a.counters(&mut v);
                v
            }
            None if outputs.starting.contains(&k) => json!({ "active": false, "starting": true }),
            None => json!({ "active": false }),
        };
        json!({
            "backend": backend,
            "record": status(OutputKind::Record),
            "stream": status(OutputKind::Stream),
        })
    }

    /// Met l'enregistrement en pause ou le reprend. Pendant la pause, aucune image
    /// ni aucun son n'est poussé : les horodatages des sorties suivent leurs propres
    /// compteurs, le fichier reprend donc sans trou ni saut.
    pub fn set_record_paused(&self, paused: bool) -> Result<Value, String> {
        let data = {
            let mut outputs = self.outputs.lock().unwrap();
            let active = outputs
                .active
                .get_mut(&OutputKind::Record)
                .ok_or("output_not_active")?;
            match (paused, active.paused_since) {
                (true, Some(_)) => return Err("output_paused".into()),
                (false, None) => return Err("output_not_paused".into()),
                (true, None) => active.paused_since = Some(Instant::now()),
                (false, Some(since)) => {
                    active.paused_total += since.elapsed();
                    active.paused_since = None;
                }
            }
            json!({
                "output": OutputKind::Record,
                "paused": paused,
                "duration_s": active.duration().as_secs_f64(),
            })
        };
        self.emit(
            if paused {
                "RecordingPaused"
            } else {
                "RecordingResumed"
            },
            data.clone(),
        );
        Ok(data)
    }

    pub fn has_active_outputs(&self) -> bool {
        !self.outputs.lock().unwrap().active.is_empty()
    }

    /// Arrête les sorties en échec et prévient les clients.
    fn fail_outputs(&self, failed: Vec<(OutputKind, String)>) {
        for (kind, error) in failed {
            tracing::warn!(?kind, %error, "sortie en échec");
            // La boucle de rendu et le mixeur détectent la même erreur : seul le
            // thread qui retire effectivement la sortie prévient les clients.
            if self.stop_output(kind).is_ok() {
                self.emit("OutputFailed", json!({ "output": kind, "error": error }));
            }
        }
    }

    /// Lance la boucle de rendu et le mixeur audio, et démarre les captures des
    /// sources déjà présentes.
    pub fn spawn_render_loop(self: &Arc<Self>) {
        self.sync_captures();
        self.spawn_audio_mixer();
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
                        engine.compositor.lock().unwrap().render_with(
                            canvas,
                            &scene,
                            frame,
                            &*engine.media,
                            &mut buf,
                        );
                        let mut failed = Vec::new();
                        {
                            let mut outputs = engine.outputs.lock().unwrap();
                            for (kind, active) in outputs.active.iter_mut() {
                                // Image rendue avec un autre canvas que celui de la
                                // sortie (course au démarrage) : on ne la pousse pas.
                                if active.canvas != canvas || active.paused_since.is_some() {
                                    continue;
                                }
                                if let Err(e) = active.output.push_frame(&buf) {
                                    failed.push((*kind, e.to_string()));
                                    continue;
                                }
                                // Le son est poussé par le mixeur, calé sur ce compteur.
                                active.frames += 1;
                            }
                        }
                        engine.fail_outputs(failed);
                    }
                    let period = Duration::from_secs_f64(1.0 / canvas.fps.max(1) as f64);
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else {
                        // En retard d'au moins une période : on saute les images
                        // manquées au lieu de les rattraper, mais les sorties avancent
                        // leur horloge d'autant, pour que la durée enregistrée reste
                        // celle du temps réel (sinon une machine lente produit un
                        // fichier plus court et le son en avance est jeté).
                        let missed = ((now - next).as_secs_f64() / period.as_secs_f64()) as u64;
                        if missed > 0 {
                            *engine.frame.lock().unwrap() += missed;
                            if has_outputs {
                                engine.skip_frames(missed, canvas, &buf);
                            }
                            next += period.mul_f64(missed as f64);
                        }
                    }
                }
            })
            .expect("thread de rendu");
    }

    /// Images que la boucle de rendu n'a pas pu produire à temps : chaque sortie
    /// active (non en pause, au même canvas) avance son compteur d'autant.
    fn skip_frames(&self, n: u64, canvas: Canvas, last: &[u8]) {
        let mut failed = Vec::new();
        {
            let mut outputs = self.outputs.lock().unwrap();
            for (kind, active) in outputs.active.iter_mut() {
                if active.canvas != canvas || active.paused_since.is_some() {
                    continue;
                }
                if let Err(e) = active.output.skip_frames(n, last) {
                    failed.push((*kind, e.to_string()));
                    continue;
                }
                active.frames += n;
            }
        }
        self.fail_outputs(failed);
    }

    /// Pousse un bloc mixé dans l'entrée audio de chaque sortie active, calé sur
    /// l'horodatage vidéo de la sortie (voir [`mixer::plan_block`]).
    fn push_mixed_audio(&self, block: &[i16]) {
        let mut failed = Vec::new();
        {
            let mut outputs = self.outputs.lock().unwrap();
            for (kind, active) in outputs.active.iter_mut() {
                let Some(audio) = &active.audio else {
                    continue;
                };
                if active.paused_since.is_some() {
                    continue;
                }
                let Some(silence) =
                    mixer::plan_block(active.frames, active.canvas.fps, active.audio_pos)
                else {
                    continue;
                };
                let mut result = Ok(());
                if silence > 0 {
                    let zeros = vec![0i16; silence as usize * AUDIO_CHANNELS as usize];
                    result = audio.push_audio(&zeros);
                }
                if result.is_ok() {
                    result = audio.push_audio(block);
                }
                match result {
                    Ok(()) => active.audio_pos += silence + BLOCK_FRAMES as u64,
                    Err(e) => failed.push((*kind, e.to_string())),
                }
            }
        }
        self.fail_outputs(failed);
    }

    /// Thread du mixeur : toutes les 10 ms, tire un bloc de chaque source audio,
    /// mixe les sources audibles (scène programme, visibles, non coupées) avec leur
    /// volume, pousse le résultat dans les sorties (du silence s'il n'y a aucune
    /// source) et émet `InputLevels` environ 10 fois par seconde.
    fn spawn_audio_mixer(self: &Arc<Self>) {
        let engine = Arc::clone(self);
        std::thread::Builder::new()
            .name("audio-mixer".into())
            .spawn(move || {
                let mut bus = MixBus::new(BLOCK_SAMPLES);
                let mut input = vec![0i16; BLOCK_SAMPLES];
                let mut block = vec![0i16; BLOCK_SAMPLES];
                let mut meters: HashMap<Uuid, LevelMeter> = HashMap::new();
                let period = Duration::from_millis(u64::from(BLOCK_MS));
                let mut blocks: u64 = 0;
                let mut next = Instant::now();
                loop {
                    let settings: HashMap<Uuid, AudioSetting> = {
                        let s = engine.studio.lock().unwrap();
                        let program = s.program_scene;
                        let mut map: HashMap<Uuid, AudioSetting> = s
                            .scenes
                            .iter()
                            .flat_map(|sc| {
                                sc.sources
                                    .iter()
                                    .filter(|x| x.kind.has_audio())
                                    .map(move |x| {
                                        (
                                            x.id,
                                            AudioSetting {
                                                volume: x.volume,
                                                muted: x.muted,
                                                audible: sc.id == program && x.visible,
                                                input: false,
                                            },
                                        )
                                    })
                            })
                            .collect();
                        // Les entrées globales partent toujours dans le mix.
                        for i in &s.audio_inputs {
                            map.insert(
                                i.id,
                                AudioSetting {
                                    volume: i.volume,
                                    muted: i.muted,
                                    audible: true,
                                    input: true,
                                },
                            );
                        }
                        map
                    };
                    bus.clear();
                    for (id, media) in engine.media.audio_sources() {
                        let Some(audio) = media.audio.as_ref() else {
                            continue;
                        };
                        // Toutes les files sont vidées, même hors antenne, pour qu'une
                        // source reparte sans retard quand sa scène passe à l'antenne.
                        audio.pull(&mut input);
                        let Some(set) = settings.get(&id) else {
                            continue;
                        };
                        meters.entry(id).or_default().add(&input, set.volume);
                        if set.audible {
                            bus.add_source(&input, set.volume, set.muted);
                        }
                    }
                    bus.output(&mut block);
                    engine.push_mixed_audio(&block);
                    blocks += 1;
                    if blocks.is_multiple_of(LEVELS_EVERY_BLOCKS) {
                        meters.retain(|id, _| settings.contains_key(id));
                        engine.emit_levels(&mut meters, &settings);
                    }
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else if now - next > period * 10 {
                        // Gros retard (machine suspendue…) : on se recale. Les petits
                        // retards sont rattrapés, l'audio garde son débit moyen.
                        next = now;
                    }
                }
            })
            .expect("thread du mixeur audio");
    }

    /// Émet `InputLevels` (crête et RMS en dBFS par canal) si des clients écoutent.
    fn emit_levels(
        &self,
        meters: &mut HashMap<Uuid, LevelMeter>,
        settings: &HashMap<Uuid, AudioSetting>,
    ) {
        if meters.is_empty() || self.events.receiver_count() == 0 {
            for m in meters.values_mut() {
                m.take();
            }
            return;
        }
        let (sources, inputs) = levels_payload(meters, settings);
        self.emit(
            "InputLevels",
            json!({ "sources": sources, "inputs": inputs }),
        );
    }
}

/// Réglages audio d'une source ou d'une entrée, lus à chaque bloc de mixage.
#[derive(Debug, Clone, Copy, Default)]
struct AudioSetting {
    volume: f32,
    muted: bool,
    /// Part dans le mix : source visible de la scène programme, ou entrée globale.
    audible: bool,
    /// Entrée audio globale (`studio.mixer`).
    input: bool,
}

/// Données de `InputLevels` : sources des scènes (`sources`, clé `source_id`) et
/// entrées audio globales (`inputs`, clé `input_id`).
fn levels_payload(
    meters: &mut HashMap<Uuid, LevelMeter>,
    settings: &HashMap<Uuid, AudioSetting>,
) -> (Vec<Value>, Vec<Value>) {
    let mut sources = Vec::new();
    let mut inputs = Vec::new();
    for (id, m) in meters.iter_mut() {
        let l = m.take();
        let set = settings.get(id).copied().unwrap_or_default();
        let mut v = json!({
            "peak_db": l.peak_db.map(mixer::round_db),
            "rms_db": l.rms_db.map(mixer::round_db),
            "muted": set.muted,
            "audible": set.audible,
        });
        if set.input {
            v["input_id"] = json!(id);
            inputs.push(v);
        } else {
            v["source_id"] = json!(id);
            sources.push(v);
        }
    }
    (sources, inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Arc<Engine> {
        let canvas = Canvas {
            width: 320,
            height: 240,
            fps: 30,
        };
        Engine::new(
            Studio::new(canvas),
            std::env::temp_dir(),
            OutputConfig::default(),
        )
    }

    #[test]
    fn capture_events_are_remembered() {
        let states = Mutex::new(HashMap::new());
        let id = Uuid::new_v4();
        record_capture_state(
            &states,
            "SourceFailed",
            &json!({ "source_id": id, "kind": "camera", "error": "débranchée", "retry_in_ms": 2000 }),
        );
        let failed = states.lock().unwrap()[&id].clone();
        assert_eq!(failed["state"], "failed");
        assert_eq!(failed["error"], "débranchée");
        assert_eq!(failed["retry_in_ms"], 2000);
        record_capture_state(&states, "SourceStarted", &json!({ "source_id": id }));
        assert_eq!(states.lock().unwrap()[&id], json!({ "state": "running" }));
        // Les autres événements ne changent rien.
        record_capture_state(&states, "InputLevels", &json!({ "sources": [] }));
        assert_eq!(states.lock().unwrap().len(), 1);
    }

    #[test]
    fn changed_capture_settings_forget_old_state() {
        let srt = |url: &str| SourceKind::SrtInput { url: url.into() };
        let (a, b, gone) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut states = HashMap::new();
        let mut synced = HashMap::new();
        let first = vec![
            (a, srt("srt://0.0.0.0:9000?mode=listener")),
            (b, srt("srt://0.0.0.0:9001?mode=listener")),
            (gone, srt("srt://0.0.0.0:9002?mode=listener")),
        ];
        prune_capture_states(&mut states, &mut synced, &first);
        for id in [a, b, gone] {
            states.insert(id, json!({ "state": "running" }));
        }
        // `a` change d'URL, `b` ne change pas, `gone` est retirée.
        let second = vec![
            (a, srt("srt://0.0.0.0:9100?mode=listener")),
            (b, srt("srt://0.0.0.0:9001?mode=listener")),
        ];
        prune_capture_states(&mut states, &mut synced, &second);
        assert!(
            !states.contains_key(&a),
            "état d'avant le changement oublié"
        );
        assert_eq!(states[&b], json!({ "state": "running" }));
        assert!(!states.contains_key(&gone));
        // Le nouvel état de `a` est gardé tant que ses réglages restent les mêmes.
        states.insert(a, json!({ "state": "failed", "error": "x" }));
        prune_capture_states(&mut states, &mut synced, &second);
        assert_eq!(states[&a]["state"], "failed");
    }

    #[test]
    fn starting_slot_blocks_canvas_and_second_start() {
        let engine = engine();
        engine
            .outputs
            .lock()
            .unwrap()
            .starting
            .insert(OutputKind::Record);
        // Une sortie en démarrage n'est pas encore active pour la boucle de rendu…
        assert!(!engine.has_active_outputs());
        // … mais le canvas est gelé et l'emplacement est pris.
        let new_canvas = Canvas {
            width: 640,
            height: 360,
            fps: 60,
        };
        assert_eq!(engine.set_canvas(new_canvas).unwrap_err(), "outputs_active");
        assert_eq!(engine.studio.lock().unwrap().canvas.width, 320);
        let err = engine
            .start_output(OutputKind::Record, StartOptions::default())
            .unwrap_err();
        assert_eq!(err, "output_already_active");
        let status = engine.output_status();
        assert_eq!(status["record"]["active"], false);
        assert_eq!(status["record"]["starting"], true);
        // L'emplacement libéré, le canvas peut de nouveau changer.
        engine
            .outputs
            .lock()
            .unwrap()
            .starting
            .remove(&OutputKind::Record);
        engine.set_canvas(new_canvas).unwrap();
        assert_eq!(engine.studio.lock().unwrap().canvas, new_canvas);
    }

    /// Backend GStreamer seulement : le backend ffmpeg lance son processus sans
    /// vérifier la destination, l'échec n'arrive qu'ensuite.
    #[cfg(feature = "gstreamer")]
    #[test]
    fn failed_start_releases_slot() {
        let engine = engine();
        // Destination impossible : le démarrage échoue après la réservation.
        let opts = StartOptions {
            target: Some("/proc/interdit/enregistrement.mkv".into()),
            ..StartOptions::default()
        };
        assert!(engine.start_output(OutputKind::Record, opts).is_err());
        let outputs = engine.outputs.lock().unwrap();
        assert!(outputs.is_idle(), "l'emplacement doit être libéré");
    }

    /// La boucle de rendu et le mixeur peuvent signaler la même panne : le
    /// second signalement, qui trouve la sortie déjà retirée, n'émet rien.
    #[test]
    fn output_failure_reported_once() {
        let engine = engine();
        let mut events = engine.subscribe();
        engine.fail_outputs(vec![(OutputKind::Stream, "erreur".into())]);
        while let Ok(msg) = events.try_recv() {
            if let ServerMessage::Event { event_type, .. } = msg {
                assert_ne!(event_type, "OutputFailed", "sortie déjà retirée");
            }
        }
    }

    #[test]
    fn shutdown_refuses_new_outputs() {
        let engine = engine();
        assert!(engine.shutdown_outputs().is_empty());
        let opts = StartOptions {
            target: Some("rtmp://127.0.0.1/app/cle".into()),
            ..StartOptions::default()
        };
        assert_eq!(
            engine.start_output(OutputKind::Stream, opts).unwrap_err(),
            "shutting_down"
        );
        assert!(engine.outputs.lock().unwrap().is_idle());
    }

    #[test]
    fn levels_split_sources_and_inputs() {
        let (src, inp) = (Uuid::new_v4(), Uuid::new_v4());
        let mut meters: HashMap<Uuid, LevelMeter> = HashMap::new();
        meters.entry(src).or_default().add(&[1000, 1000], 1.0);
        meters.entry(inp).or_default().add(&[1000, 1000], 1.0);
        let settings = HashMap::from([
            (
                src,
                AudioSetting {
                    volume: 1.0,
                    muted: false,
                    audible: false,
                    input: false,
                },
            ),
            (
                inp,
                AudioSetting {
                    volume: 1.0,
                    muted: true,
                    audible: true,
                    input: true,
                },
            ),
        ]);
        let (sources, inputs) = levels_payload(&mut meters, &settings);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0]["source_id"], json!(src));
        assert_eq!(sources[0]["audible"], false);
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0]["input_id"], json!(inp));
        assert_eq!(inputs[0]["muted"], true);
        assert!(inputs[0].get("source_id").is_none());
    }
}
