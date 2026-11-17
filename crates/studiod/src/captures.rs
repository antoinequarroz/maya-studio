//! Gestionnaire de captures : un pipeline GStreamer par source de capture.
//!
//! Chaque source de capture présente dans au moins une scène a son pipeline :
//!
//! ```text
//! caméra   : <source plateforme> ! decodebin ! [branche vidéo]
//! écran    : <source plateforme> ! [branche vidéo]
//! fenêtre  : <source plateforme> ! [branche vidéo]
//! micro    : <source plateforme> ! [branche audio]
//! fichier  : uridecodebin ! [branche vidéo] + [branche audio]
//! SRT      : srtsrc ! decodebin ! [branche vidéo] + [branche audio]
//!
//! branche vidéo : queue ! videoconvert ! videoscale ! [videorate] ! appsink (RGBA, max-buffers=1, drop)
//! branche audio : queue ! audioconvert ! audioresample ! appsink (S16LE 48 kHz stéréo)
//! ```
//!
//! Les éléments propres à une plateforme sont choisis à l'exécution avec
//! `gst::ElementFactory::find` (aucun crate spécifique) : `avfvideosrc` sous macOS,
//! `mfvideosrc` / `ksvideosrc` / `d3d11screencapturesrc` / `wasapi2src` sous Windows,
//! `v4l2src` / `ximagesrc` / `pipewiresrc` / `pulsesrc` sous Linux.
//!
//! La dernière image de chaque source et son son vont dans le [`MediaStore`] ;
//! la boucle de rendu et le mixeur audio les y lisent. Un pipeline en erreur
//! émet `SourceFailed` puis redémarre avec un délai croissant (1 s à 30 s).

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::glib;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;
use serde_json::{json, Value};
use studio_core::{SourceKind, VideoFrame};
use uuid::Uuid;

use crate::devices;
use crate::media::{MediaStore, SourceMedia};
use crate::outputs::{AUDIO_CHANNELS, AUDIO_RATE};

/// Émetteur d'événements vers les clients (type, données).
pub type EmitFn = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// Premier délai avant redémarrage d'un pipeline en échec.
const BACKOFF_MIN: Duration = Duration::from_secs(1);
/// Délai maximal entre deux tentatives.
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Un pipeline qui a tourné au moins ce temps repart avec le délai minimal.
const STABLE_AFTER: Duration = Duration::from_secs(30);

/// Signal d'arrêt d'une capture, avec attente interruptible.
#[derive(Default)]
struct StopSignal {
    stopped: Mutex<bool>,
    cond: Condvar,
}

impl StopSignal {
    fn stop(&self) {
        *self.stopped.lock().unwrap() = true;
        self.cond.notify_all();
    }

    fn is_stopped(&self) -> bool {
        *self.stopped.lock().unwrap()
    }

    /// Attend `d` ; renvoie vrai si l'arrêt a été demandé entre-temps.
    fn wait(&self, d: Duration) -> bool {
        let guard = self.stopped.lock().unwrap();
        let (guard, _) = self.cond.wait_timeout_while(guard, d, |s| !*s).unwrap();
        *guard
    }
}

struct Capture {
    kind: SourceKind,
    stop: Arc<StopSignal>,
    thread: Option<JoinHandle<()>>,
}

/// Lance, arrête et surveille les pipelines de capture.
pub struct CaptureManager {
    media: Arc<MediaStore>,
    emit: EmitFn,
    captures: Mutex<HashMap<Uuid, Capture>>,
}

impl CaptureManager {
    pub fn new(media: Arc<MediaStore>, emit: EmitFn) -> Self {
        Self {
            media,
            emit,
            captures: Mutex::new(HashMap::new()),
        }
    }

    /// Met les captures en accord avec les sources présentes dans les scènes :
    /// démarre les nouvelles, arrête celles qui ont disparu, relance celles dont
    /// les réglages ont changé.
    pub fn sync(&self, wanted: &[(Uuid, SourceKind)]) {
        let mut captures = self.captures.lock().unwrap();
        let stale: Vec<Uuid> = captures
            .iter()
            .filter(|(id, c)| !wanted.iter().any(|(w, k)| w == *id && *k == c.kind))
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            if let Some(c) = captures.remove(&id) {
                self.media.remove(id);
                c.stop.stop();
                // Le thread démonte son pipeline seul : on ne bloque pas l'appelant.
                drop(c.thread);
                tracing::info!(source = %id, "capture arrêtée");
            }
        }
        for (id, kind) in wanted {
            if captures.contains_key(id) || !kind.is_capture() {
                continue;
            }
            let media = Arc::new(SourceMedia::new(kind.has_audio()));
            self.media.insert(*id, Arc::clone(&media));
            let stop = Arc::new(StopSignal::default());
            let ctx = CaptureContext {
                id: *id,
                kind: kind.clone(),
                media,
                stop: Arc::clone(&stop),
                emit: Arc::clone(&self.emit),
            };
            let thread = std::thread::Builder::new()
                .name(format!("capture-{}", &id.to_string()[..8]))
                .spawn(move || ctx.supervise())
                .map_err(|e| tracing::error!(source = %id, error = %e, "thread de capture"))
                .ok();
            captures.insert(
                *id,
                Capture {
                    kind: kind.clone(),
                    stop,
                    thread,
                },
            );
        }
    }

    /// Sources dont la capture tourne (ou redémarre).
    pub fn running(&self) -> Vec<Uuid> {
        self.captures.lock().unwrap().keys().copied().collect()
    }

    /// Arrête toutes les captures et attend la fin de leurs threads.
    pub fn stop_all(&self) {
        let all: Vec<(Uuid, Capture)> = self.captures.lock().unwrap().drain().collect();
        for (id, c) in &all {
            self.media.remove(*id);
            c.stop.stop();
        }
        for (_, mut c) in all {
            if let Some(t) = c.thread.take() {
                let _ = t.join();
            }
        }
    }
}

impl Drop for CaptureManager {
    fn drop(&mut self) {
        for c in self.captures.lock().unwrap().values() {
            c.stop.stop();
        }
    }
}

/// Tout ce dont a besoin le thread d'une capture.
struct CaptureContext {
    id: Uuid,
    kind: SourceKind,
    media: Arc<SourceMedia>,
    stop: Arc<StopSignal>,
    emit: EmitFn,
}

/// Fin d'une exécution du pipeline.
enum RunEnd {
    /// Arrêt demandé.
    Stopped,
    /// Échec : on redémarre après un délai.
    Failed(String),
}

impl CaptureContext {
    /// Boucle de supervision : lance le pipeline, le relance après un échec.
    fn supervise(self) {
        let mut backoff = BACKOFF_MIN;
        loop {
            if self.stop.is_stopped() {
                break;
            }
            let started = Instant::now();
            let error = match self.run_once() {
                RunEnd::Stopped => break,
                RunEnd::Failed(e) => e,
            };
            if self.stop.is_stopped() {
                break;
            }
            if started.elapsed() >= STABLE_AFTER {
                backoff = BACKOFF_MIN;
            }
            tracing::warn!(
                source = %self.id,
                kind = kind_name(&self.kind),
                %error,
                retry_in_ms = backoff.as_millis() as u64,
                "capture en échec"
            );
            // Plus d'image figée ni de son résiduel pendant la panne.
            *self.media.video.lock().unwrap() = None;
            if let Some(a) = &self.media.audio {
                a.clear();
            }
            (self.emit)(
                "SourceFailed",
                json!({
                    "source_id": self.id,
                    "kind": kind_name(&self.kind),
                    "error": error,
                    "retry_in_ms": backoff.as_millis() as u64,
                }),
            );
            if self.stop.wait(backoff) {
                break;
            }
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
        tracing::debug!(source = %self.id, "fin du thread de capture");
    }

    fn run_once(&self) -> RunEnd {
        let pipeline = match build_pipeline(&self.kind, &self.media) {
            Ok(p) => p,
            Err(e) => return RunEnd::Failed(e),
        };
        let end = self.run_pipeline(&pipeline);
        let _ = pipeline.set_state(gst::State::Null);
        end
    }

    fn run_pipeline(&self, pipeline: &gst::Pipeline) -> RunEnd {
        let Some(bus) = pipeline.bus() else {
            return RunEnd::Failed("pipeline sans bus".into());
        };
        if let Err(e) = pipeline.set_state(gst::State::Playing) {
            // Le détail (périphérique absent, affichage inaccessible…) arrive sur le bus.
            let detail = bus
                .timed_pop_filtered(
                    gst::ClockTime::from_mseconds(500),
                    &[gst::MessageType::Error],
                )
                .and_then(|m| match m.view() {
                    gst::MessageView::Error(err) => Some(error_text(err)),
                    _ => None,
                });
            return RunEnd::Failed(
                detail.unwrap_or_else(|| format!("démarrage du pipeline impossible : {e}")),
            );
        }
        let looping = matches!(self.kind, SourceKind::MediaFile { looping: true, .. });
        let is_file = matches!(self.kind, SourceKind::MediaFile { .. });
        let mut announced = false;
        loop {
            if self.stop.is_stopped() {
                return RunEnd::Stopped;
            }
            let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) else {
                continue;
            };
            match msg.view() {
                gst::MessageView::Error(err) => return RunEnd::Failed(error_text(err)),
                gst::MessageView::Warning(w) => {
                    tracing::debug!(source = %self.id, warning = %w.error(), "avertissement de capture");
                }
                gst::MessageView::Eos(_) if looping => {
                    // Bouclage : retour au début, sans redémarrer le pipeline.
                    if let Err(e) = pipeline.seek_simple(
                        gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                        gst::ClockTime::ZERO,
                    ) {
                        return RunEnd::Failed(format!("retour au début impossible : {e}"));
                    }
                }
                gst::MessageView::Eos(_) if is_file => {
                    // Fin du fichier sans bouclage : la dernière image reste affichée.
                    tracing::info!(source = %self.id, "fin du fichier média");
                }
                gst::MessageView::Eos(_) => {
                    return RunEnd::Failed("fin de flux inattendue".into());
                }
                gst::MessageView::StateChanged(s)
                    if !announced
                        && s.current() == gst::State::Playing
                        && msg.src() == Some(pipeline.upcast_ref::<gst::Object>()) =>
                {
                    announced = true;
                    tracing::info!(source = %self.id, kind = kind_name(&self.kind), "capture démarrée");
                    (self.emit)(
                        "SourceStarted",
                        json!({ "source_id": self.id, "kind": kind_name(&self.kind) }),
                    );
                }
                _ => {}
            }
        }
    }
}

fn error_text(err: &gst::message::Error) -> String {
    let src = err
        .src()
        .map(|s| s.name().to_string())
        .unwrap_or_else(|| "?".into());
    format!("{src} : {}", err.error())
}

/// Nom du type de source, comme dans l'API (`camera`, `media_file`…).
pub fn kind_name(kind: &SourceKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v["kind"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Premier élément disponible parmi les candidats.
pub fn first_available(candidates: &[&'static str]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|n| gst::ElementFactory::find(n).is_some())
}

/// Sources caméra par plateforme, par ordre de préférence.
pub fn camera_candidates(os: &str) -> &'static [&'static str] {
    match os {
        "macos" => &["avfvideosrc"],
        "windows" => &["mfvideosrc", "ksvideosrc"],
        "linux" => &["v4l2src"],
        _ => &[],
    }
}

/// Sources de capture d'écran par plateforme. Sous Linux, PipeWire passe en
/// premier en session Wayland (X11 n'y donne pas accès à l'écran).
pub fn display_candidates(os: &str, wayland_only: bool) -> &'static [&'static str] {
    match os {
        "macos" => &["avfvideosrc"],
        "windows" => &["d3d11screencapturesrc"],
        "linux" if wayland_only => &["pipewiresrc", "ximagesrc"],
        "linux" => &["ximagesrc", "pipewiresrc"],
        _ => &[],
    }
}

/// Entrées audio par plateforme.
pub fn audio_candidates(os: &str) -> &'static [&'static str] {
    match os {
        "macos" => &["osxaudiosrc"],
        "windows" => &["wasapi2src", "wasapisrc"],
        "linux" => &["pulsesrc", "pipewiresrc", "alsasrc"],
        _ => &[],
    }
}

fn wayland_only() -> bool {
    std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn make(factory: &str) -> Result<gst::Element, String> {
    gst::ElementFactory::make(factory)
        .build()
        .map_err(|_| format!("élément GStreamer « {factory} » introuvable"))
}

fn make_first(candidates: &[&'static str], what: &str) -> Result<gst::Element, String> {
    let name = first_available(candidates).ok_or_else(|| {
        if candidates.is_empty() {
            format!("{what} : non pris en charge sur cette plateforme")
        } else {
            format!(
                "{what} : aucun élément disponible parmi {} (plugin GStreamer manquant)",
                candidates.join(", ")
            )
        }
    })?;
    make(name)
}

fn factory_name(el: &gst::Element) -> String {
    el.factory()
        .map(|f| f.name().to_string())
        .unwrap_or_default()
}

/// Fixe une propriété à partir de texte (le type est déduit de la propriété).
fn set_prop_str(el: &gst::Element, name: &str, value: &str) -> Result<(), String> {
    let pspec = el
        .find_property(name)
        .ok_or_else(|| format!("« {} » n'a pas de propriété « {name} »", factory_name(el)))?;
    let v = glib::Value::deserialize(value, pspec.value_type())
        .map_err(|_| format!("valeur invalide pour « {name} » : {value}"))?;
    el.set_property_from_value(name, &v);
    Ok(())
}

fn has_prop(el: &gst::Element, name: &str) -> bool {
    el.find_property(name).is_some()
}

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Élément source d'une caméra : le périphérique découvert s'il est connu,
/// sinon l'élément de la plateforme avec l'identifiant pris comme chemin ou index.
fn camera_element(device_id: Option<&str>) -> Result<gst::Element, String> {
    if let Some(id) = device_id {
        if let Some(dev) = devices::find(devices::VIDEO_CLASS, id) {
            return dev
                .create_element(None)
                .map_err(|e| format!("caméra « {id} » : {e}"));
        }
    }
    let el = make_first(camera_candidates(std::env::consts::OS), "caméra")?;
    if let Some(id) = device_id {
        let factory = factory_name(&el);
        let prop = match factory.as_str() {
            "v4l2src" => "device",
            "avfvideosrc" => "device-index",
            _ if is_number(id) && has_prop(&el, "device-index") => "device-index",
            _ if has_prop(&el, "device-path") => "device-path",
            _ => "device-name",
        };
        set_prop_str(&el, prop, id)?;
    }
    Ok(el)
}

fn display_element(display_id: Option<&str>) -> Result<gst::Element, String> {
    let el = make_first(
        display_candidates(std::env::consts::OS, wayland_only()),
        "capture d'écran",
    )?;
    match factory_name(&el).as_str() {
        "avfvideosrc" => {
            set_prop_str(&el, "capture-screen", "true")?;
            if let Some(id) = display_id {
                set_prop_str(&el, "device-index", id)?;
            }
        }
        "d3d11screencapturesrc" => {
            if let Some(id) = display_id {
                set_prop_str(&el, "monitor-index", id)?;
            }
        }
        "ximagesrc" => {
            // Sans les « damage » X, l'image entière est relue à chaque fois : plus
            // simple et plus fiable avec les compositeurs.
            set_prop_str(&el, "use-damage", "false")?;
            match display_id {
                Some(id) if is_number(id) => set_prop_str(&el, "screen-num", id)?,
                Some(id) => set_prop_str(&el, "display-name", id)?,
                None => {}
            }
        }
        "pipewiresrc" => {
            if let Some(id) = display_id {
                let prop = if has_prop(&el, "target-object") {
                    "target-object"
                } else {
                    "path"
                };
                set_prop_str(&el, prop, id)?;
            }
        }
        _ => {}
    }
    Ok(el)
}

fn window_element(window_id: Option<&str>) -> Result<gst::Element, String> {
    let os = std::env::consts::OS;
    let candidates: &[&'static str] = match os {
        "windows" => &["d3d11screencapturesrc"],
        "linux" => &["ximagesrc"],
        // avfvideosrc ne sait capturer que des écrans entiers.
        _ => &[],
    };
    let el = make_first(candidates, "capture de fenêtre")?;
    let id = window_id.ok_or("capture de fenêtre : window_id est requis")?;
    let prop = match factory_name(&el).as_str() {
        "ximagesrc" => "xid",
        _ => "window-handle",
    };
    if !has_prop(&el, prop) {
        return Err(format!(
            "capture de fenêtre non prise en charge par cette version de « {} »",
            factory_name(&el)
        ));
    }
    set_prop_str(&el, prop, id)?;
    if factory_name(&el) == "ximagesrc" {
        set_prop_str(&el, "use-damage", "false")?;
    }
    Ok(el)
}

fn audio_element(device_id: Option<&str>) -> Result<gst::Element, String> {
    if let Some(id) = device_id {
        if let Some(dev) = devices::find(devices::AUDIO_CLASS, id) {
            return dev
                .create_element(None)
                .map_err(|e| format!("entrée audio « {id} » : {e}"));
        }
    }
    let el = make_first(audio_candidates(std::env::consts::OS), "entrée audio")?;
    if let Some(id) = device_id {
        let prop = if has_prop(&el, "device") {
            "device"
        } else if has_prop(&el, "target-object") {
            "target-object"
        } else {
            return Err(format!("entrée audio « {id} » introuvable"));
        };
        set_prop_str(&el, prop, id)?;
    }
    Ok(el)
}

/// URI d'un fichier média : URI telle quelle, ou chemin local rendu absolu.
fn media_uri(path: &str) -> Result<String, String> {
    if path.contains("://") {
        return Ok(path.to_string());
    }
    let p = std::path::Path::new(path);
    let abs = std::fs::canonicalize(p).map_err(|e| format!("fichier « {path} » : {e}"))?;
    glib::filename_to_uri(&abs, None)
        .map(|u| u.to_string())
        .map_err(|e| format!("fichier « {path} » : {e}"))
}

/// Réglages de la branche vidéo.
#[derive(Clone, Copy, Default)]
struct VideoOpts {
    /// Synchronisation sur l'horloge (lecture d'un fichier en temps réel).
    sync: bool,
    /// Source en direct : la file jette les vieilles images plutôt que d'attendre.
    live: bool,
    size: Option<(u32, u32)>,
    fps: Option<u32>,
}

fn video_branch(media: &Arc<SourceMedia>, opts: VideoOpts) -> Result<gst::Bin, String> {
    let queue = if opts.live {
        "queue max-size-buffers=2 max-size-bytes=0 max-size-time=0 leaky=downstream"
    } else {
        "queue"
    };
    let rate = if opts.fps.is_some() {
        "videorate ! "
    } else {
        ""
    };
    let description = format!(
        "{queue} ! videoconvert ! videoscale ! {rate}capsfilter name=vcaps ! appsink name=vsink"
    );
    let bin = gst::parse::bin_from_description(&description, true)
        .map_err(|e| format!("branche vidéo : {e}"))?;
    let mut caps = gst::Caps::builder("video/x-raw").field("format", "RGBA");
    if let Some((w, h)) = opts.size {
        caps = caps
            .field("width", w as i32)
            .field("height", h as i32)
            .field("pixel-aspect-ratio", gst::Fraction::new(1, 1));
    }
    if let Some(fps) = opts.fps {
        caps = caps.field("framerate", gst::Fraction::new(fps as i32, 1));
    }
    bin.by_name("vcaps")
        .ok_or("vcaps absent")?
        .set_property("caps", caps.build());
    let sink = bin
        .by_name("vsink")
        .and_then(|e| e.downcast::<gst_app::AppSink>().ok())
        .ok_or("vsink n'est pas un appsink")?;
    sink.set_max_buffers(1);
    sink.set_drop(true);
    sink.set_sync(opts.sync);
    let media = Arc::clone(media);
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                if let Some(frame) = sample_to_frame(&sample) {
                    media.set_frame(frame);
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    Ok(bin)
}

/// Copie une image RGBA GStreamer (lignes éventuellement espacées) en image contiguë.
fn sample_to_frame(sample: &gst::Sample) -> Option<VideoFrame> {
    let info = gst_video::VideoInfo::from_caps(sample.caps()?).ok()?;
    let buffer = sample.buffer()?;
    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info).ok()?;
    let (w, h) = (info.width() as usize, info.height() as usize);
    let stride = frame.plane_stride()[0] as usize;
    let src = frame.plane_data(0).ok()?;
    let row = w * 4;
    let mut data = Vec::with_capacity(row * h);
    for y in 0..h {
        data.extend_from_slice(src.get(y * stride..y * stride + row)?);
    }
    VideoFrame::new(w as u32, h as u32, data)
}

fn audio_branch(media: &Arc<SourceMedia>, sync: bool) -> Result<gst::Bin, String> {
    let bin = gst::parse::bin_from_description(
        "queue ! audioconvert ! audioresample ! capsfilter name=acaps ! appsink name=asink",
        true,
    )
    .map_err(|e| format!("branche audio : {e}"))?;
    let caps = gst::Caps::builder("audio/x-raw")
        .field("format", "S16LE")
        .field("layout", "interleaved")
        .field("rate", AUDIO_RATE as i32)
        .field("channels", AUDIO_CHANNELS as i32)
        .build();
    bin.by_name("acaps")
        .ok_or("acaps absent")?
        .set_property("caps", caps);
    let sink = bin
        .by_name("asink")
        .and_then(|e| e.downcast::<gst_app::AppSink>().ok())
        .ok_or("asink n'est pas un appsink")?;
    sink.set_sync(sync);
    let media = Arc::clone(media);
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let (Some(buffer), Some(audio)) = (sample.buffer(), media.audio.as_ref()) else {
                    return Ok(gst::FlowSuccess::Ok);
                };
                if let Ok(map) = buffer.map_readable() {
                    let samples: Vec<i16> = map
                        .chunks_exact(2)
                        .map(|b| i16::from_le_bytes([b[0], b[1]]))
                        .collect();
                    audio.push(&samples);
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    Ok(bin)
}

/// Ajoute une branche au pipeline et relie `src_pad` à son entrée.
fn attach_branch(
    pipeline: &gst::Pipeline,
    src_pad: &gst::Pad,
    bin: gst::Bin,
) -> Result<(), String> {
    pipeline
        .add(&bin)
        .map_err(|e| format!("ajout de la branche : {e}"))?;
    bin.sync_state_with_parent()
        .map_err(|e| format!("démarrage de la branche : {e}"))?;
    let sink_pad = bin.static_pad("sink").ok_or("branche sans entrée")?;
    src_pad
        .link(&sink_pad)
        .map(|_| ())
        .map_err(|e| format!("liaison de la branche : {e:?}"))
}

/// Relie les pads d'un décodeur dynamique (decodebin, uridecodebin) : la
/// première piste vidéo et la première piste audio ont chacune leur branche.
fn connect_decoder(
    pipeline: &gst::Pipeline,
    decoder: &gst::Element,
    media: &Arc<SourceMedia>,
    video: Option<VideoOpts>,
    audio_sync: Option<bool>,
) {
    let pipeline_weak = pipeline.downgrade();
    let media = Arc::clone(media);
    let linked = Arc::new(Mutex::new((false, false)));
    decoder.connect_pad_added(move |_, pad| {
        let Some(pipeline) = pipeline_weak.upgrade() else {
            return;
        };
        let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
        let Some(name) = caps.structure(0).map(|s| s.name().to_string()) else {
            return;
        };
        let mut linked = linked.lock().unwrap();
        let result = if name.starts_with("video/") && !linked.0 {
            let Some(opts) = video else { return };
            linked.0 = true;
            video_branch(&media, opts).and_then(|b| attach_branch(&pipeline, pad, b))
        } else if name.starts_with("audio/") && !linked.1 {
            let Some(sync) = audio_sync else { return };
            linked.1 = true;
            audio_branch(&media, sync).and_then(|b| attach_branch(&pipeline, pad, b))
        } else {
            // Pistes supplémentaires (sous-titres, 2ᵉ piste audio) : ignorées.
            return;
        };
        if let Err(e) = result {
            gst::element_error!(pipeline, gst::CoreError::Negotiation, ("{}", e));
        }
    });
}

fn link_static(pipeline: &gst::Pipeline, src: &gst::Element, bin: gst::Bin) -> Result<(), String> {
    pipeline
        .add(&bin)
        .map_err(|e| format!("ajout de la branche : {e}"))?;
    src.link(&bin)
        .map_err(|e| format!("liaison de {} : {e}", factory_name(src)))
}

/// Construit le pipeline d'une source de capture (sans le démarrer).
pub fn build_pipeline(
    kind: &SourceKind,
    media: &Arc<SourceMedia>,
) -> Result<gst::Pipeline, String> {
    gst::init().map_err(|e| format!("initialisation de GStreamer impossible : {e}"))?;
    let pipeline = gst::Pipeline::new();
    let add = |el: &gst::Element| {
        pipeline
            .add(el)
            .map_err(|e| format!("ajout de {} : {e}", factory_name(el)))
    };
    let live = VideoOpts {
        live: true,
        ..VideoOpts::default()
    };
    match kind {
        SourceKind::Camera {
            device_id,
            width,
            height,
            fps,
        } => {
            let src = camera_element(device_id.as_deref())?;
            // decodebin laisse passer la vidéo brute et décode le MJPEG des webcams.
            let decode = make("decodebin")?;
            add(&src)?;
            add(&decode)?;
            src.link(&decode)
                .map_err(|e| format!("liaison de {} : {e}", factory_name(&src)))?;
            let opts = VideoOpts {
                size: width.zip(*height).filter(|(w, h)| *w > 0 && *h > 0),
                fps: fps.filter(|f| *f > 0),
                ..live
            };
            connect_decoder(&pipeline, &decode, media, Some(opts), None);
        }
        SourceKind::DisplayCapture { display_id } => {
            let src = display_element(display_id.as_deref())?;
            add(&src)?;
            link_static(&pipeline, &src, video_branch(media, live)?)?;
        }
        SourceKind::WindowCapture { window_id } => {
            let src = window_element(window_id.as_deref())?;
            add(&src)?;
            link_static(&pipeline, &src, video_branch(media, live)?)?;
        }
        SourceKind::AudioInput { device_id } => {
            let src = audio_element(device_id.as_deref())?;
            add(&src)?;
            link_static(&pipeline, &src, audio_branch(media, false)?)?;
        }
        SourceKind::MediaFile { path, .. } => {
            let decode = make("uridecodebin")?;
            decode.set_property("uri", media_uri(path)?);
            add(&decode)?;
            // Lecture au rythme de l'horloge : le fichier défile en temps réel.
            let opts = VideoOpts {
                sync: true,
                ..VideoOpts::default()
            };
            connect_decoder(&pipeline, &decode, media, Some(opts), Some(true));
        }
        SourceKind::SrtInput { url } => {
            let src = make("srtsrc")?;
            src.set_property("uri", url);
            let decode = make("decodebin")?;
            add(&src)?;
            add(&decode)?;
            src.link(&decode)
                .map_err(|e| format!("liaison de srtsrc : {e}"))?;
            connect_decoder(&pipeline, &decode, media, Some(live), Some(false));
        }
        SourceKind::Color { .. } | SourceKind::Image { .. } | SourceKind::TestPattern => {
            return Err("cette source n'a pas de capture".into());
        }
    }
    Ok(pipeline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_candidates() {
        assert_eq!(camera_candidates("linux"), ["v4l2src"]);
        assert_eq!(camera_candidates("macos"), ["avfvideosrc"]);
        assert_eq!(camera_candidates("windows"), ["mfvideosrc", "ksvideosrc"]);
        assert_eq!(display_candidates("linux", false)[0], "ximagesrc");
        assert_eq!(display_candidates("linux", true)[0], "pipewiresrc");
        assert_eq!(
            display_candidates("windows", false),
            ["d3d11screencapturesrc"]
        );
        assert_eq!(audio_candidates("macos"), ["osxaudiosrc"]);
        assert!(camera_candidates("plan9").is_empty());
    }

    #[test]
    fn kind_names() {
        assert_eq!(
            kind_name(&SourceKind::MediaFile {
                path: "a".into(),
                looping: false
            }),
            "media_file"
        );
        assert_eq!(
            kind_name(&SourceKind::AudioInput { device_id: None }),
            "audio_input"
        );
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        let media = Arc::new(SourceMedia::new(true));
        let err = build_pipeline(
            &SourceKind::MediaFile {
                path: "/nulle/part/clip.mp4".into(),
                looping: false,
            },
            &media,
        )
        .unwrap_err();
        assert!(err.contains("clip.mp4"), "{err}");
        assert!(build_pipeline(&SourceKind::TestPattern, &media).is_err());
    }

    /// Les chemins Linux se construisent même sans caméra ni écran ; l'échec
    /// arrive au démarrage, sous forme d'erreur.
    #[test]
    fn linux_capture_pipelines_build_without_devices() {
        if std::env::consts::OS != "linux" {
            return;
        }
        gst::init().unwrap();
        let media = Arc::new(SourceMedia::new(false));
        if first_available(&["v4l2src"]).is_some() {
            let p = build_pipeline(
                &SourceKind::Camera {
                    device_id: Some("/dev/video-inexistant".into()),
                    width: Some(640),
                    height: Some(360),
                    fps: Some(30),
                },
                &media,
            )
            .expect("pipeline caméra");
            assert!(p.set_state(gst::State::Playing).is_err());
            let _ = p.set_state(gst::State::Null);
        }
        if first_available(&["ximagesrc"]).is_some() {
            let p = build_pipeline(&SourceKind::DisplayCapture { display_id: None }, &media)
                .expect("pipeline écran");
            let _ = p.set_state(gst::State::Null);
            let p = build_pipeline(
                &SourceKind::WindowCapture {
                    window_id: Some("0x1234".into()),
                },
                &media,
            )
            .expect("pipeline fenêtre");
            let _ = p.set_state(gst::State::Null);
        }
        let err = build_pipeline(&SourceKind::WindowCapture { window_id: None }, &media);
        assert!(err.is_err());
    }
}
