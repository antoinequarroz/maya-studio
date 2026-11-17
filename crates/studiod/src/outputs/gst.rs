//! Backend de sortie GStreamer, in-process.
//!
//! Pipeline construit pour chaque sortie :
//!
//! ```text
//! appsrc vsrc (RGBA) ! queue ! videoconvert ! <encodeur H.264> ! h264parse ! queue ! mux.
//! appsrc asrc (S16 48 kHz stéréo) ! queue ! audioconvert ! audioresample ! <AAC> ! aacparse ! queue ! mux.
//! <mux> ! <sink>
//! ```
//!
//! - enregistrement : `mp4mux` (éventuellement fragmenté) ou `matroskamux`, puis `filesink` ;
//! - RTMP(S) : `flvmux streamable=true ! rtmp2sink` ;
//! - SRT : `mpegtsmux ! srtsink`.
//!
//! Les horodatages sont calculés à partir de compteurs (images, échantillons) :
//! ils sont exacts et monotones, et l'audio reste calé sur la vidéo.
//! `push_frame` ne bloque jamais : la file de l'appsrc est bornée et une image
//! qui ne rentre pas est perdue (compteur `dropped_frames`).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

use super::{
    redact_target, AudioSink, Output, OutputKind, OutputSettings, RecordFormat, StreamProtocol,
    AUDIO_CHANNELS, AUDIO_RATE,
};
use crate::encoders::{self, PropValue};

/// Délai maximal d'attente de l'EOS à l'arrêt.
const EOS_TIMEOUT: Duration = Duration::from_secs(5);
/// Images RGBA au plus en attente dans l'appsrc vidéo.
const VIDEO_QUEUE_FRAMES: u64 = 8;
/// Audio au plus en attente dans l'appsrc audio (1 s).
const AUDIO_QUEUE_BYTES: u64 = (AUDIO_RATE * AUDIO_CHANNELS * 2) as u64;
/// Marge laissée aux muxers en direct pour attendre l'audio et la vidéo.
const MUX_LATENCY_NS: u64 = 500_000_000;

/// Conteneur du pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4 { fragmented: bool },
    Mkv,
    Flv,
    MpegTs,
}

/// Élément final du pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkKind {
    File,
    Rtmp,
    Srt,
}

/// Plan d'un pipeline, indépendant de GStreamer (testable sans rien lancer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelinePlan {
    pub encoder: String,
    pub aac_encoder: String,
    pub container: Container,
    pub sink: SinkKind,
}

impl PipelinePlan {
    pub fn new(settings: &OutputSettings, encoder: &str, aac: &str) -> Result<Self, String> {
        let (container, sink) = match settings.kind {
            OutputKind::Record => (
                match settings.format {
                    RecordFormat::Mp4 => Container::Mp4 {
                        fragmented: settings.fragmented,
                    },
                    RecordFormat::Mkv => Container::Mkv,
                },
                SinkKind::File,
            ),
            OutputKind::Stream => match StreamProtocol::from_url(&settings.target) {
                Some(StreamProtocol::Rtmp) => (Container::Flv, SinkKind::Rtmp),
                Some(StreamProtocol::Srt) => (Container::MpegTs, SinkKind::Srt),
                None => return Err("unsupported_url".into()),
            },
        };
        Ok(Self {
            encoder: encoder.to_string(),
            aac_encoder: aac.to_string(),
            container,
            sink,
        })
    }

    /// Description `gst-launch` du pipeline. Elle ne contient aucune donnée
    /// fournie par l'utilisateur : chemin et URL sont fixés ensuite par propriété.
    pub fn description(&self) -> String {
        let mux = match self.container {
            Container::Mp4 { fragmented: false } => "mp4mux".to_string(),
            // Fragments d'une seconde : le fichier reste lisible après un plantage.
            Container::Mp4 { fragmented: true } => "mp4mux fragment-duration=1000".to_string(),
            Container::Mkv => "matroskamux".to_string(),
            Container::Flv => "flvmux streamable=true".to_string(),
            // 7 paquets TS par datagramme (1316 octets), la taille usuelle en SRT.
            Container::MpegTs => "mpegtsmux alignment=7".to_string(),
        };
        let sink = match self.sink {
            SinkKind::File => "filesink",
            SinkKind::Rtmp => "rtmp2sink",
            SinkKind::Srt => "srtsink",
        };
        format!(
            "appsrc name=vsrc ! queue name=vqueue max-size-buffers=4 max-size-bytes=0 max-size-time=0 \
             ! videoconvert ! {enc} name=venc ! h264parse config-interval=-1 ! queue ! mux. \
             appsrc name=asrc ! queue name=aqueue ! audioconvert ! audioresample \
             ! {aac} name=aenc ! aacparse ! queue ! mux. \
             {mux} name=mux ! {sink} name=sink",
            enc = self.encoder,
            aac = self.aac_encoder,
        )
    }
}

/// État partagé avec le thread qui surveille le bus.
#[derive(Default)]
struct BusFlags {
    eos: bool,
    error: Option<String>,
}

#[derive(Default)]
struct BusState {
    flags: Mutex<BusFlags>,
    changed: Condvar,
    quit: AtomicBool,
}

impl BusState {
    fn error(&self) -> Option<String> {
        self.flags.lock().unwrap().error.clone()
    }

    /// Attend l'EOS ou une erreur ; renvoie l'état final.
    fn wait(&self, timeout: Duration) -> (bool, Option<String>) {
        let guard = self.flags.lock().unwrap();
        let (guard, _) = self
            .changed
            .wait_timeout_while(guard, timeout, |f| !f.eos && f.error.is_none())
            .unwrap();
        (guard.eos, guard.error.clone())
    }
}

fn watch_bus(bus: gst::Bus, state: Arc<BusState>, target: String) {
    while !state.quit.load(Ordering::Relaxed) {
        let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) else {
            continue;
        };
        match msg.view() {
            gst::MessageView::Eos(_) => {
                state.flags.lock().unwrap().eos = true;
                state.changed.notify_all();
            }
            gst::MessageView::Error(err) => {
                let src = err
                    .src()
                    .map(|s| s.name().to_string())
                    .unwrap_or_else(|| "?".into());
                let error = format!("{src} : {}", err.error());
                tracing::warn!(
                    %target,
                    %error,
                    debug = err.debug().as_deref().unwrap_or(""),
                    "erreur GStreamer"
                );
                let mut f = state.flags.lock().unwrap();
                if f.error.is_none() {
                    f.error = Some(error);
                }
                drop(f);
                state.changed.notify_all();
            }
            gst::MessageView::Warning(w) => {
                tracing::warn!(%target, warning = %w.error(), "avertissement GStreamer");
            }
            _ => {}
        }
    }
}

/// Horodatage exact de l'unité `n` (image ou échantillon) à `rate` unités par seconde.
fn timestamp(n: u64, rate: u64) -> gst::ClockTime {
    let ns = u128::from(n) * 1_000_000_000u128 / u128::from(rate.max(1));
    gst::ClockTime::from_nseconds(ns as u64)
}

/// Entrée audio d'une sortie GStreamer (appsrc S16LE 48 kHz stéréo).
pub struct GstAudioSink {
    src: gst_app::AppSrc,
    /// Échantillons (par canal) déjà poussés : horloge de la piste audio.
    position: Mutex<u64>,
    dropped: AtomicU64,
    bus: Arc<BusState>,
}

impl GstAudioSink {
    /// Blocs audio perdus parce que la file était pleine.
    pub fn dropped_blocks(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl AudioSink for GstAudioSink {
    fn push_audio(&self, interleaved: &[i16]) -> std::io::Result<()> {
        if let Some(e) = self.bus.error() {
            return Err(std::io::Error::other(e));
        }
        if !interleaved.len().is_multiple_of(AUDIO_CHANNELS as usize) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "nombre d'échantillons non multiple du nombre de canaux",
            ));
        }
        if interleaved.is_empty() {
            return Ok(());
        }
        let frames = (interleaved.len() / AUDIO_CHANNELS as usize) as u64;
        let mut pos = self.position.lock().unwrap();
        let start = *pos;
        // La position avance même si le bloc est perdu : l'audio suivant reste à sa place.
        *pos += frames;
        let bytes = interleaved.len() as u64 * 2;
        if self.src.current_level_bytes() + bytes > AUDIO_QUEUE_BYTES {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        let mut data = Vec::with_capacity(bytes as usize);
        for s in interleaved {
            data.extend_from_slice(&s.to_le_bytes());
        }
        let mut buffer = gst::Buffer::from_mut_slice(data);
        {
            let b = buffer.get_mut().expect("tampon neuf");
            let pts = timestamp(start, u64::from(AUDIO_RATE));
            b.set_pts(pts);
            b.set_duration(timestamp(start + frames, u64::from(AUDIO_RATE)) - pts);
        }
        match self.src.push_buffer(buffer) {
            Ok(_) | Err(gst::FlowError::Flushing) => Ok(()),
            Err(e) => Err(std::io::Error::other(format!("audio : {e:?}"))),
        }
    }
}

pub struct GstOutput {
    pipeline: gst::Pipeline,
    video: gst_app::AppSrc,
    audio: Arc<GstAudioSink>,
    frame_size: usize,
    fps: u64,
    frames: u64,
    dropped: u64,
    bus: Arc<BusState>,
    bus_thread: Option<JoinHandle<()>>,
    target: String,
    encoder: String,
}

impl GstOutput {
    pub fn start(settings: &OutputSettings) -> Result<Self, String> {
        gst::init().map_err(|e| format!("initialisation de GStreamer impossible : {e}"))?;
        let encoder = encoders::select(&settings.encoder)?;
        let aac = encoders::select_aac()?;
        let plan = PipelinePlan::new(settings, &encoder, &aac)?;
        let description = plan.description();
        tracing::debug!(%description, "pipeline de sortie");
        let pipeline = gst::parse::launch(&description)
            .map_err(|e| format!("pipeline invalide : {e}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "le pipeline n'est pas un gst::Pipeline".to_string())?;
        let by_name = |name: &str| {
            pipeline
                .by_name(name)
                .ok_or_else(|| format!("élément « {name} » absent du pipeline"))
        };

        let canvas = settings.canvas;
        let frame_size = canvas.width as usize * canvas.height as usize * 4;

        // Vidéo.
        let video = by_name("vsrc")?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| "vsrc n'est pas un appsrc".to_string())?;
        let video_caps = gst_video::VideoInfo::builder(
            gst_video::VideoFormat::Rgba,
            canvas.width,
            canvas.height,
        )
        .fps(gst::Fraction::new(canvas.fps.max(1) as i32, 1))
        .build()
        .and_then(|i| i.to_caps())
        .map_err(|e| format!("caps vidéo : {e}"))?;
        configure_appsrc(&video, &video_caps, frame_size as u64 * VIDEO_QUEUE_FRAMES);
        encoders::apply(
            &by_name("venc")?,
            &encoders::video_tuning(&encoder, settings.bitrate_kbps, canvas.fps),
        );

        // Audio.
        let asrc = by_name("asrc")?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| "asrc n'est pas un appsrc".to_string())?;
        let audio_caps = gst::Caps::builder("audio/x-raw")
            .field("format", "S16LE")
            .field("layout", "interleaved")
            .field("rate", AUDIO_RATE as i32)
            .field("channels", AUDIO_CHANNELS as i32)
            .field("channel-mask", gst::Bitmask::new(0x3))
            .build();
        configure_appsrc(&asrc, &audio_caps, AUDIO_QUEUE_BYTES);
        encoders::apply(&by_name("aenc")?, &encoders::audio_tuning());

        // Muxer et destination.
        encoders::set_if_exists(
            &by_name("mux")?,
            "latency",
            &PropValue::Uint(MUX_LATENCY_NS),
        );
        let sink = by_name("sink")?;
        match plan.sink {
            SinkKind::File => sink.set_property("location", &settings.target),
            SinkKind::Rtmp => sink.set_property("location", &settings.target),
            SinkKind::Srt => sink.set_property("uri", &settings.target),
        }

        let bus_state = Arc::new(BusState::default());
        let bus = pipeline.bus().ok_or("pipeline sans bus")?;
        let bus_thread = {
            let state = Arc::clone(&bus_state);
            let target = redact_target(&settings.target);
            std::thread::Builder::new()
                .name("sortie-bus".into())
                .spawn(move || watch_bus(bus, state, target))
                .map_err(|e| format!("thread du bus : {e}"))?
        };

        let audio = Arc::new(GstAudioSink {
            src: asrc,
            position: Mutex::new(0),
            dropped: AtomicU64::new(0),
            bus: Arc::clone(&bus_state),
        });
        let output = Self {
            pipeline,
            video,
            audio,
            frame_size,
            fps: u64::from(canvas.fps.max(1)),
            frames: 0,
            dropped: 0,
            bus: bus_state,
            bus_thread: Some(bus_thread),
            target: settings.target.clone(),
            encoder,
        };

        if let Err(e) = output.pipeline.set_state(gst::State::Playing) {
            // Le détail arrive sur le bus juste après l'échec.
            let (_, detail) = output.bus.wait(Duration::from_millis(500));
            return Err(detail.unwrap_or_else(|| format!("démarrage du pipeline impossible : {e}")));
        }
        tracing::info!(
            target = %redact_target(&output.target),
            encoder = %output.encoder,
            audio = %aac,
            "sortie GStreamer démarrée"
        );
        Ok(output)
    }

    fn shutdown(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        self.bus.quit.store(true, Ordering::Relaxed);
        if let Some(t) = self.bus_thread.take() {
            let _ = t.join();
        }
    }
}

fn configure_appsrc(src: &gst_app::AppSrc, caps: &gst::Caps, max_bytes: u64) {
    src.set_caps(Some(caps));
    src.set_format(gst::Format::Time);
    src.set_is_live(true);
    src.set_do_timestamp(false);
    src.set_stream_type(gst_app::AppStreamType::Stream);
    src.set_max_bytes(max_bytes);
    // Jamais de blocage dans push_buffer : on gère nous-mêmes le débordement.
    encoders::set_if_exists(src, "block", &PropValue::Bool(false));
    encoders::set_if_exists(src, "max-buffers", &PropValue::Uint(0));
}

impl Output for GstOutput {
    fn push_frame(&mut self, rgba: &[u8]) -> std::io::Result<()> {
        if let Some(e) = self.bus.error() {
            return Err(std::io::Error::other(e));
        }
        if rgba.len() != self.frame_size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "taille d'image inattendue : {} octets au lieu de {}",
                    rgba.len(),
                    self.frame_size
                ),
            ));
        }
        let n = self.frames;
        // Le compteur avance même si l'image est perdue : le temps reste réel.
        self.frames += 1;
        if self.video.current_level_bytes() + self.frame_size as u64
            > self.frame_size as u64 * VIDEO_QUEUE_FRAMES
        {
            self.dropped += 1;
            if self.dropped == 1 || self.dropped.is_multiple_of(100) {
                tracing::warn!(
                    target = %redact_target(&self.target),
                    dropped = self.dropped,
                    "la sortie ne suit pas (encodeur ou réseau), images perdues"
                );
            }
            return Ok(());
        }
        let mut buffer = gst::Buffer::from_mut_slice(rgba.to_vec());
        {
            let b = buffer.get_mut().expect("tampon neuf");
            let pts = timestamp(n, self.fps);
            b.set_pts(pts);
            b.set_duration(timestamp(n + 1, self.fps) - pts);
        }
        match self.video.push_buffer(buffer) {
            Ok(_) => Ok(()),
            Err(gst::FlowError::Flushing) => {
                self.dropped += 1;
                Ok(())
            }
            Err(e) => Err(std::io::Error::other(format!("vidéo : {e:?}"))),
        }
    }

    fn audio_sink(&self) -> Option<Arc<dyn AudioSink>> {
        Some(Arc::clone(&self.audio) as Arc<dyn AudioSink>)
    }

    fn stop(mut self: Box<Self>) -> Result<(), String> {
        let result = match self.bus.error() {
            Some(e) => Err(e),
            None => {
                let _ = self.video.end_of_stream();
                let _ = self.audio.src.end_of_stream();
                match self.bus.wait(EOS_TIMEOUT) {
                    (_, Some(e)) => Err(e),
                    (true, None) => Ok(()),
                    (false, None) => Err(format!(
                        "fin de flux non reçue en {} s : la sortie peut être incomplète",
                        EOS_TIMEOUT.as_secs()
                    )),
                }
            }
        };
        self.shutdown();
        let audio_dropped = self.audio.dropped_blocks();
        tracing::info!(
            target = %redact_target(&self.target),
            frames = self.frames,
            dropped = self.dropped,
            audio_dropped,
            ok = result.is_ok(),
            "sortie GStreamer arrêtée"
        );
        result
    }

    fn target(&self) -> &str {
        &self.target
    }

    fn encoder(&self) -> &str {
        &self.encoder
    }

    fn dropped_frames(&self) -> u64 {
        self.dropped
    }
}

impl Drop for GstOutput {
    fn drop(&mut self) {
        if self.bus_thread.is_some() {
            self.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outputs::EncoderChoice;
    use studio_core::Canvas;

    fn settings(kind: OutputKind, target: &str, format: RecordFormat) -> OutputSettings {
        OutputSettings {
            kind,
            canvas: Canvas {
                width: 320,
                height: 240,
                fps: 30,
            },
            target: target.into(),
            format,
            fragmented: false,
            bitrate_kbps: 1000,
            encoder: EncoderChoice::Auto,
        }
    }

    #[test]
    fn record_mp4_description() {
        let plan = PipelinePlan::new(
            &settings(OutputKind::Record, "a.mp4", RecordFormat::Mp4),
            "openh264enc",
            "avenc_aac",
        )
        .unwrap();
        assert_eq!(plan.container, Container::Mp4 { fragmented: false });
        let d = plan.description();
        assert!(d.starts_with("appsrc name=vsrc ! queue"));
        assert!(d.contains("videoconvert ! openh264enc name=venc ! h264parse"));
        assert!(d.contains("appsrc name=asrc"));
        assert!(d.contains("avenc_aac name=aenc ! aacparse"));
        assert!(d.contains("mp4mux name=mux ! filesink name=sink"));
        // Le chemin n'est jamais interpolé dans la description.
        assert!(!d.contains("a.mp4"));
    }

    #[test]
    fn record_variants() {
        let mut s = settings(OutputKind::Record, "a.mp4", RecordFormat::Mp4);
        s.fragmented = true;
        let d = PipelinePlan::new(&s, "x", "y").unwrap().description();
        assert!(d.contains("mp4mux fragment-duration=1000 name=mux"));
        let s = settings(OutputKind::Record, "a.mkv", RecordFormat::Mkv);
        let d = PipelinePlan::new(&s, "x", "y").unwrap().description();
        assert!(d.contains("matroskamux name=mux ! filesink"));
    }

    #[test]
    fn stream_descriptions() {
        for url in ["rtmp://h/app/key", "rtmps://h/app/key"] {
            let s = settings(OutputKind::Stream, url, RecordFormat::Mp4);
            let plan = PipelinePlan::new(&s, "vtenc_h264_hw", "voaacenc").unwrap();
            assert_eq!(plan.sink, SinkKind::Rtmp);
            assert!(plan
                .description()
                .contains("flvmux streamable=true name=mux ! rtmp2sink name=sink"));
        }
        let s = settings(
            OutputKind::Stream,
            "srt://127.0.0.1:9000",
            RecordFormat::Mp4,
        );
        let d = PipelinePlan::new(&s, "nvh264enc", "avenc_aac")
            .unwrap()
            .description();
        assert!(d.contains("mpegtsmux alignment=7 name=mux ! srtsink name=sink"));
        let s = settings(OutputKind::Stream, "udp://x", RecordFormat::Mp4);
        assert_eq!(
            PipelinePlan::new(&s, "a", "b").unwrap_err(),
            "unsupported_url"
        );
    }

    #[test]
    fn descriptions_parse() {
        gst::init().unwrap();
        for (kind, target, format) in [
            (OutputKind::Record, "a.mp4", RecordFormat::Mp4),
            (OutputKind::Record, "a.mkv", RecordFormat::Mkv),
            (OutputKind::Stream, "rtmp://h/a/k", RecordFormat::Mp4),
            (OutputKind::Stream, "srt://h:1", RecordFormat::Mp4),
        ] {
            let plan =
                PipelinePlan::new(&settings(kind, target, format), "openh264enc", "avenc_aac")
                    .unwrap();
            gst::parse::launch(&plan.description()).unwrap_or_else(|e| panic!("{target} : {e}"));
        }
    }

    #[test]
    fn timestamps_exact_and_monotonic() {
        assert_eq!(timestamp(30, 30), gst::ClockTime::SECOND);
        assert_eq!(timestamp(48_000, 48_000), gst::ClockTime::SECOND);
        let mut last = gst::ClockTime::ZERO;
        for n in 1..1000 {
            let t = timestamp(n, 30);
            assert!(t > last);
            last = t;
        }
    }
}
