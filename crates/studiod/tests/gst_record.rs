//! Test d'intégration du backend GStreamer : enregistre ~2 s d'images avec
//! openh264enc et une piste audio silencieuse, puis vérifie le fichier avec ffprobe.
#![cfg(feature = "gstreamer")]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use studio_core::Canvas;
use studiod::outputs::gst::GstOutput;
use studiod::outputs::{
    audio_samples_for_frame, EncoderChoice, Output, OutputKind, OutputSettings, RecordFormat,
    AUDIO_CHANNELS,
};

const W: u32 = 320;
const H: u32 = 240;
const FPS: u32 = 30;

fn record(path: &Path, format: RecordFormat, fragmented: bool) -> u64 {
    let settings = OutputSettings {
        kind: OutputKind::Record,
        canvas: Canvas {
            width: W,
            height: H,
            fps: FPS,
        },
        target: path.to_string_lossy().into_owned(),
        format,
        fragmented,
        bitrate_kbps: 1500,
        encoder: EncoderChoice::Named("openh264enc".into()),
    };
    let mut out: Box<dyn Output> = Box::new(GstOutput::start(&settings).expect("démarrage"));
    assert_eq!(out.encoder(), "openh264enc");
    let audio = out.audio_sink().expect("entrée audio");
    let mut frame = vec![0u8; (W * H * 4) as usize];
    let period = Duration::from_secs_f64(1.0 / FPS as f64);
    let start = Instant::now();
    for n in 0..u64::from(FPS * 2) {
        // Dégradé qui bouge, pour que l'encodeur ait quelque chose à coder.
        for (i, px) in frame.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&[(i as u64 + n * 4) as u8, (n * 3) as u8, 128, 255]);
        }
        out.push_frame(&frame).expect("push_frame");
        let samples = audio_samples_for_frame(n, FPS) * AUDIO_CHANNELS as usize;
        audio.push_audio(&vec![0i16; samples]).expect("push_audio");
        // Cadence temps réel, comme la boucle de rendu.
        let next = period * (n as u32 + 1);
        if let Some(wait) = next.checked_sub(start.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    let dropped = out.dropped_frames();
    drop(audio);
    out.stop().expect("arrêt propre avec EOS");
    dropped
}

fn ffprobe(path: &Path) -> Value {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
        ])
        .arg(path)
        .output()
        .expect("ffprobe est nécessaire pour ce test (paquet ffmpeg)");
    assert!(
        out.status.success(),
        "ffprobe a échoué : {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("JSON ffprobe")
}

fn check(path: &Path) {
    let info = ffprobe(path);
    let streams = info["streams"].as_array().expect("streams");
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .expect("piste vidéo");
    assert_eq!(video["codec_name"], "h264");
    assert_eq!(video["width"], W);
    assert_eq!(video["height"], H);
    let audio = streams
        .iter()
        .find(|s| s["codec_type"] == "audio")
        .expect("piste audio");
    assert_eq!(audio["codec_name"], "aac");
    assert_eq!(audio["sample_rate"], "48000");
    assert_eq!(audio["channels"], 2);
    let duration: f64 = info["format"]["duration"]
        .as_str()
        .expect("durée")
        .parse()
        .unwrap();
    assert!(
        (1.7..=2.4).contains(&duration),
        "durée inattendue : {duration} s"
    );
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("studiod-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn record_mp4_openh264_with_aac() {
    let path = temp_path("enregistrement.mp4");
    let dropped = record(&path, RecordFormat::Mp4, false);
    assert_eq!(dropped, 0, "aucune image ne doit être perdue à 320x240");
    check(&path);
}

#[test]
fn record_fragmented_mp4() {
    let path = temp_path("fragmente.mp4");
    record(&path, RecordFormat::Mp4, true);
    check(&path);
}

#[test]
fn record_mkv() {
    let path = temp_path("enregistrement.mkv");
    record(&path, RecordFormat::Mkv, false);
    check(&path);
}

/// Bout en bout dans le moteur : boucle de rendu, requêtes API, silence audio.
#[test]
fn engine_records_through_api() {
    use studio_core::{SourceKind, Studio, Transform};
    use studiod::engine::{Engine, OutputConfig};

    let canvas = Canvas {
        width: W,
        height: H,
        fps: FPS,
    };
    let mut studio = Studio::new(canvas);
    studio
        .add_source(
            "Scène 1",
            "Mire",
            SourceKind::TestPattern,
            Some(Transform::full(canvas)),
        )
        .unwrap();
    let config = OutputConfig {
        encoder: EncoderChoice::Named("openh264enc".into()),
        ..OutputConfig::default()
    };
    let engine = Engine::new(studio, std::env::temp_dir(), config);
    engine.spawn_render_loop();
    let path = temp_path("moteur.mkv");
    let started = studiod::api::handle(
        &engine,
        "StartRecording",
        serde_json::json!({ "path": path, "bitrate_kbps": 2000 }),
    )
    .expect("StartRecording");
    assert_eq!(started["encoder"], "openh264enc");
    assert_eq!(started["format"], "mkv");
    std::thread::sleep(Duration::from_millis(2000));
    let status = studiod::api::handle(&engine, "GetOutputStatus", Value::Null).unwrap();
    assert_eq!(status["record"]["active"], true);
    assert_eq!(status["record"]["encoder"], "openh264enc");
    assert!(status["record"]["dropped_frames"].is_u64());
    let stopped = studiod::api::handle(&engine, "StopRecording", Value::Null).unwrap();
    assert_eq!(stopped["finalized"], true);
    check(&path);
}
