//! Tests d'intégration des captures : fichier média (vidéo + son) jusqu'à
//! l'enregistrement, mute, et échecs propres des captures sans périphérique.
#![cfg(feature = "gstreamer")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use studio_core::{Canvas, FrameProvider, Studio};
use studiod::engine::{Engine, OutputConfig};
use studiod::outputs::EncoderChoice;
use suite_protocol::ServerMessage;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Sous Windows, `Command` n'ajoute `.exe` qu'aux noms sans extension : « gst-launch-1.0 »
/// en a une (« .0 »), il faut donc le nom complet.
const GST_LAUNCH: &str = if cfg!(windows) {
    "gst-launch-1.0.exe"
} else {
    "gst-launch-1.0"
};

const W: u32 = 320;
const H: u32 = 240;
const FPS: u32 = 30;

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("studiod-captures-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Fichier de test de 3 s : mire animée + la 440 Hz, créé une fois avec gst-launch.
fn test_clip() -> &'static Path {
    static CLIP: OnceLock<PathBuf> = OnceLock::new();
    CLIP.get_or_init(|| {
        let path = temp_dir().join("clip.mp4");
        let status = Command::new(GST_LAUNCH)
            .args(["-q", "-e"])
            .args(
                format!(
                    "videotestsrc num-buffers=90 pattern=ball ! video/x-raw,width=320,height=240,framerate=30/1 \
                     ! videoconvert ! {} ! h264parse ! queue ! mp4mux name=mux ! filesink location={} \
                     audiotestsrc num-buffers=141 wave=sine freq=440 volume=0.5 \
                     ! audio/x-raw,rate=48000,channels=2 ! audioconvert ! avenc_aac ! aacparse ! queue ! mux.",
                    studiod::encoders::test_encoder(),
                    // Barres obliques : gst-launch lit « \\ » comme un échappement (chemins Windows).
                    path.display().to_string().replace('\\', "/")
                )
                .split_whitespace(),
            )
            .status()
            .expect("gst-launch-1.0 est nécessaire pour ce test");
        assert!(status.success(), "création du fichier de test impossible");
        path
    })
}

fn engine() -> Arc<Engine> {
    let canvas = Canvas {
        width: W,
        height: H,
        fps: FPS,
    };
    let config = OutputConfig {
        encoder: EncoderChoice::Named(studiod::encoders::test_encoder().into()),
        ..OutputConfig::default()
    };
    let engine = Engine::new(Studio::new(canvas), temp_dir(), config);
    engine.spawn_render_loop();
    engine
}

fn call(engine: &Engine, request: &str, data: Value) -> Value {
    studiod::api::handle(engine, request, data)
        .unwrap_or_else(|e| panic!("{request} : {} ({})", e.0, e.1))
}

fn add_clip(engine: &Engine, name: &str) -> Uuid {
    let added = call(
        engine,
        "AddSource",
        json!({ "scene": "Scène 1", "name": name, "kind": "media_file",
                "path": test_clip(), "loop": true }),
    );
    added["source"]["id"].as_str().unwrap().parse().unwrap()
}

/// Attend que la source ait une image (pipeline démarré et préroulé).
fn wait_for_frame(engine: &Engine, id: Uuid) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while engine.media().frame(id).is_none() {
        assert!(Instant::now() < deadline, "aucune image reçue du fichier");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Attend un événement correspondant au prédicat.
fn wait_event(
    rx: &mut broadcast::Receiver<ServerMessage>,
    timeout: Duration,
    mut pred: impl FnMut(&str, &Value) -> bool,
) -> Option<Value> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match rx.try_recv() {
            Ok(ServerMessage::Event {
                event_type, data, ..
            }) => {
                if pred(&event_type, &data) {
                    return Some(data);
                }
            }
            Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    None
}

fn record(engine: &Engine, name: &str) -> PathBuf {
    let path = temp_dir().join(name);
    let _ = std::fs::remove_file(&path);
    call(
        engine,
        "StartRecording",
        json!({ "path": path, "bitrate_kbps": 1500 }),
    );
    std::thread::sleep(Duration::from_secs(2));
    let stopped = call(engine, "StopRecording", Value::Null);
    assert_eq!(stopped["finalized"], true, "{stopped}");
    path
}

fn ffprobe_streams(path: &Path) -> Vec<Value> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_streams"])
        .arg(path)
        .output()
        .expect("ffprobe est nécessaire pour ce test");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["streams"].as_array().unwrap().clone()
}

/// Volume moyen de la piste audio (dB), mesuré par le filtre volumedetect de ffmpeg.
fn mean_volume(path: &Path) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(path)
        .args(["-map", "0:a", "-af", "volumedetect", "-f", "null", "-"])
        .output()
        .expect("ffmpeg est nécessaire pour ce test");
    let log = String::from_utf8_lossy(&out.stderr);
    let line = log
        .lines()
        .find(|l| l.contains("mean_volume:"))
        .unwrap_or_else(|| panic!("volumedetect sans résultat :\n{log}"));
    line.split("mean_volume:")
        .nth(1)
        .unwrap()
        .trim()
        .trim_end_matches("dB")
        .trim()
        .parse()
        .unwrap()
}

fn check_av(path: &Path) {
    let streams = ffprobe_streams(path);
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .expect("piste vidéo");
    assert_eq!(video["codec_name"], "h264");
    assert_eq!(video["width"], W);
    let audio = streams
        .iter()
        .find(|s| s["codec_type"] == "audio")
        .expect("piste audio");
    assert_eq!(audio["codec_name"], "aac");
}

#[test]
fn media_file_recorded_with_video_and_audio() {
    let engine = engine();
    let mut events = engine.subscribe();
    let id = add_clip(&engine, "Clip");
    wait_for_frame(&engine, id);
    assert!(
        wait_event(&mut events, Duration::from_secs(5), |t, d| {
            t == "SourceStarted" && d["source_id"] == id.to_string()
        })
        .is_some(),
        "SourceStarted attendu"
    );

    // Le compositeur dessine l'image du fichier dans le programme (plus de noir).
    let shot = call(&engine, "GetScreenshot", json!({ "format": "png" }));
    let b64 = shot["image"].as_str().unwrap().split(',').nth(1).unwrap();
    use base64::Engine as _;
    let png = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgb8();
    assert!(
        img.pixels().any(|p| p.0 != [0, 0, 0]),
        "l'image du fichier doit apparaître dans le programme"
    );

    // VU-mètre : le la à -6 dBFS est bien mesuré.
    let levels = wait_event(&mut events, Duration::from_secs(5), |t, d| {
        t == "InputLevels"
            && d["sources"].as_array().unwrap().iter().any(|s| {
                s["source_id"] == id.to_string() && s["peak_db"][0].as_f64().unwrap() > -20.0
            })
    });
    let levels = levels.expect("InputLevels avec un niveau audible");
    let entry = &levels["sources"][0];
    assert!(entry["rms_db"][0].as_f64().unwrap() < entry["peak_db"][0].as_f64().unwrap());
    assert_eq!(entry["audible"], true);

    let path = record(&engine, "fichier-media.mkv");
    check_av(&path);
    let mean = mean_volume(&path);
    assert!(mean > -60.0, "audio silencieux : mean_volume = {mean} dB");
    assert!(mean < -3.0, "audio saturé : mean_volume = {mean} dB");

    call(
        &engine,
        "RemoveSource",
        json!({ "scene": "Scène 1", "source": "Clip" }),
    );
    assert!(engine.running_captures().is_empty());
    assert!(engine.media().frame(id).is_none());
    engine.shutdown_captures();
}

#[test]
fn muted_media_file_records_silence() {
    let engine = engine();
    let id = add_clip(&engine, "Clip muet");
    call(
        &engine,
        "SetSourceMuted",
        json!({ "scene": "Scène 1", "source": "Clip muet", "muted": true }),
    );
    wait_for_frame(&engine, id);
    let path = record(&engine, "fichier-muet.mkv");
    check_av(&path);
    let mean = mean_volume(&path);
    assert!(
        mean < -80.0,
        "le son coupé doit donner du silence : {mean} dB"
    );
    engine.shutdown_captures();
}

/// Sans caméra ni écran (conteneur), les captures échouent proprement :
/// `SourceFailed` avec un délai de nouvelle tentative, pas de panique.
#[test]
fn missing_devices_fail_gracefully() {
    let engine = engine();
    let mut events = engine.subscribe();
    let camera = call(
        &engine,
        "AddSource",
        json!({ "scene": "Scène 1", "name": "Caméra", "kind": "camera",
                "device_id": "/dev/video-inexistant", "width": 640, "height": 360 }),
    );
    let camera_id = camera["source"]["id"].as_str().unwrap().to_string();
    let failed = wait_event(&mut events, Duration::from_secs(10), |t, d| {
        t == "SourceFailed" && d["source_id"] == camera_id
    })
    .expect("SourceFailed pour la caméra absente");
    assert_eq!(failed["kind"], "camera");
    assert!(failed["retry_in_ms"].as_u64().unwrap() >= 1000);
    assert!(!failed["error"].as_str().unwrap().is_empty());

    // Capture d'écran sans serveur X : échec propre aussi. Ce cas n'existe que sous Linux :
    // sur macOS et Windows, l'écran est accessible sans DISPLAY.
    if cfg!(target_os = "linux")
        && std::env::var_os("DISPLAY").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
    {
        let display = call(
            &engine,
            "AddSource",
            json!({ "scene": "Scène 1", "name": "Écran", "kind": "display_capture" }),
        );
        let display_id = display["source"]["id"].as_str().unwrap().to_string();
        let failed = wait_event(&mut events, Duration::from_secs(10), |t, d| {
            t == "SourceFailed" && d["source_id"] == display_id
        })
        .expect("SourceFailed pour l'écran inaccessible");
        assert_eq!(failed["kind"], "display_capture");
    }

    // Le rendu continue : une capture d'écran du programme fonctionne toujours.
    call(&engine, "GetScreenshot", json!({}));
    assert_eq!(
        engine.running_captures().len(),
        engine.studio.lock().unwrap().program().sources.len()
    );
    engine.shutdown_captures();
    assert!(engine.running_captures().is_empty());
}

/// Fichier en boucle : au-delà de sa durée (3 s), les images continuent d'arriver.
#[test]
fn media_file_loops() {
    let engine = engine();
    let mut events = engine.subscribe();
    let id = add_clip(&engine, "Boucle");
    wait_for_frame(&engine, id);
    std::thread::sleep(Duration::from_millis(3500));
    let a = engine
        .media()
        .frame(id)
        .expect("image après la fin du fichier");
    std::thread::sleep(Duration::from_millis(300));
    let b = engine.media().frame(id).expect("image");
    assert!(
        !Arc::ptr_eq(&a, &b),
        "les images doivent continuer après le bouclage"
    );
    assert!(
        wait_event(&mut events, Duration::from_millis(10), |t, _| t
            == "SourceFailed")
        .is_none(),
        "le bouclage ne doit pas être un échec"
    );
    engine.shutdown_captures();
}

/// Port UDP libre pour un test SRT.
fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Émetteur SRT de test (mire MPEG-2 en MPEG-TS, une image clé toutes les 12
/// images : un récepteur qui arrive en cours de flux décode vite) lancé avec gst-launch.
struct SrtSender(std::process::Child);

impl SrtSender {
    fn start(uri: &str) -> Self {
        let child = Command::new(GST_LAUNCH)
            .arg("-q")
            .args(
                format!(
                    "videotestsrc is-live=true pattern=ball ! video/x-raw,width=320,height=240,framerate=30/1 \
                     ! videoconvert ! avenc_mpeg2video ! mpegvideoparse ! mpegtsmux ! srtsink uri={uri}"
                )
                .split_whitespace(),
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("gst-launch-1.0 est nécessaire pour ce test");
        Self(child)
    }
}

impl Drop for SrtSender {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Changer les réglages d'une entrée SRT en écoute relance la capture sur le
/// même port : l'ancien pipeline doit l'avoir libéré avant que le nouveau
/// l'ouvre, sinon « Cannot bind » et un `SourceFailed` parasite.
#[test]
fn srt_listener_restart_reuses_port() {
    let engine = engine();
    let mut events = engine.subscribe();
    let port = free_udp_port();
    let added = call(
        &engine,
        "AddSource",
        json!({ "scene": "Scène 1", "name": "SRT", "kind": "srt_input",
                "url": format!("srt://:{port}?mode=listener") }),
    );
    let id = added["source"]["id"].as_str().unwrap().to_string();
    // Le premier pipeline écoute (en attente d'un émetteur).
    std::thread::sleep(Duration::from_millis(500));
    call(
        &engine,
        "UpdateSource",
        json!({ "scene": "Scène 1", "source": "SRT", "kind": "srt_input",
                "url": format!("srt://:{port}?mode=listener&latency=200") }),
    );
    // Pas d'émetteur : aucun SourceStarted ne doit précéder les données.
    assert!(
        wait_event(&mut events, Duration::from_millis(1500), |t, d| {
            (t == "SourceFailed" || t == "SourceStarted") && d["source_id"] == id
        })
        .is_none(),
        "ni échec (port occupé) ni démarrage sans données"
    );
    let _sender = SrtSender::start(&format!("srt://127.0.0.1:{port}"));
    let mut first = None;
    wait_event(&mut events, Duration::from_secs(10), |t, d| {
        let hit = (t == "SourceFailed" || t == "SourceStarted") && d["source_id"] == id;
        if hit {
            first = Some(t.to_string());
        }
        hit
    });
    assert_eq!(
        first.as_deref(),
        Some("SourceStarted"),
        "SourceStarted attendu après la relance, sans échec"
    );
    engine.shutdown_captures();
}

/// Entrée SRT en appel : un émetteur qui disparaît (ou un hôte injoignable)
/// donne `SourceFailed` et efface l'image, au lieu d'une image figée.
#[test]
fn srt_caller_reports_lost_stream() {
    let engine = engine();
    let mut events = engine.subscribe();
    let port = free_udp_port();
    let sender = SrtSender::start(&format!("srt://:{port}?mode=listener"));
    std::thread::sleep(Duration::from_millis(500));
    let added = call(
        &engine,
        "AddSource",
        json!({ "scene": "Scène 1", "name": "SRT", "kind": "srt_input",
                "url": format!("srt://127.0.0.1:{port}?mode=caller") }),
    );
    let id: Uuid = added["source"]["id"].as_str().unwrap().parse().unwrap();
    let ids = id.to_string();
    assert!(
        wait_event(&mut events, Duration::from_secs(10), |t, d| {
            t == "SourceStarted" && d["source_id"] == ids
        })
        .is_some(),
        "SourceStarted attendu"
    );
    wait_for_frame(&engine, id);
    drop(sender);
    let failed = wait_event(&mut events, Duration::from_secs(20), |t, d| {
        t == "SourceFailed" && d["source_id"] == ids
    })
    .expect("SourceFailed après la disparition de l'émetteur");
    assert_eq!(failed["kind"], "srt_input");
    assert!(
        engine.media().frame(id).is_none(),
        "plus d'image figée pendant la panne"
    );
    engine.shutdown_captures();
}
