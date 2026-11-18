//! Arrêt de `studiod` par signal ou par fermeture de l'entrée standard (sidecar lancé
//! avec `--exit-on-stdin-eof`) : l'enregistrement MP4 (non fragmenté, le format par
//! défaut) doit être finalisé, sinon le fichier est illisible (atome moov absent).
#![cfg(all(unix, feature = "gstreamer"))]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use suite_protocol::ClientMessage;
use tokio_tungstenite::tungstenite::Message;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Tue le processus `studiod` si le test s'arrête avant lui (assertion ratée) :
/// sans cela, un échec laisse un moteur orphelin qui tourne indéfiniment.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Ok(None) = self.0.try_wait() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

async fn connect(
    addr: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match tokio_tungstenite::connect_async(format!("ws://{addr}")).await {
            Ok((ws, _)) => return ws,
            Err(e) if Instant::now() > deadline => panic!("connexion à studiod : {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// Façon d'arrêter le moteur.
#[derive(Clone, Copy)]
enum Stop {
    /// Signal POSIX (`INT`, `TERM`).
    Signal(&'static str),
    /// Fermeture de l'entrée standard, comme quand l'app parente disparaît.
    StdinEof,
}

impl Stop {
    fn label(self) -> &'static str {
        match self {
            Stop::Signal(s) => s,
            Stop::StdinEof => "stdin",
        }
    }
}

async fn run_stop_test(stop: Stop) {
    let dir = std::env::temp_dir().join(format!(
        "studiod-signal-{}-{}",
        std::process::id(),
        stop.label()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let addr = format!("127.0.0.1:{}", free_port());
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_studiod"));
    cmd.args(["--listen", &addr, "--width", "320", "--height", "240"])
        .args([
            "--encoder",
            studiod::encoders::test_encoder(),
            "--recordings",
        ])
        .arg(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Stop::StdinEof = stop {
        cmd.arg("--exit-on-stdin-eof").stdin(Stdio::piped());
    }
    let mut guard = KillOnDrop(cmd.spawn().expect("lancement de studiod"));
    let child = &mut guard.0;

    let mut ws = connect(&addr).await;
    let identify = ClientMessage::Identify {
        token: None,
        name: Some("test".into()),
        app: None,
        events: false,
    };
    ws.send(Message::Text(
        serde_json::to_string(&identify).unwrap().into(),
    ))
    .await
    .unwrap();
    let request = ClientMessage::Request {
        id: "1".into(),
        request_type: "StartRecording".into(),
        data: json!({}),
    };
    ws.send(Message::Text(
        serde_json::to_string(&request).unwrap().into(),
    ))
    .await
    .unwrap();
    // Attente de la réponse à StartRecording.
    let response = loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("réponse à StartRecording")
            .expect("connexion fermée")
            .unwrap();
        if let Message::Text(t) = msg {
            let v: Value = serde_json::from_str(&t).unwrap();
            if v["op"] == "response" {
                break v;
            }
        }
    };
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["data"]["format"], "mp4");
    tokio::time::sleep(Duration::from_secs(2)).await;

    match stop {
        Stop::Signal(signal) => {
            let status = Command::new("kill")
                .arg(format!("-{signal}"))
                .arg(child.id().to_string())
                .status()
                .unwrap();
            assert!(status.success());
        }
        Stop::StdinEof => drop(child.stdin.take()),
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let exit = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("studiod ne s'est pas arrêté après {}", stop.label());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(exit.success(), "code de sortie : {exit:?}");

    let file = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "mp4"))
        .expect("fichier d'enregistrement");
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_streams"])
        .args(["-show_format"])
        .arg(&file)
        .output()
        .expect("ffprobe est nécessaire pour ce test (paquet ffmpeg)");
    assert!(
        out.status.success(),
        "MP4 non finalisé : {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let info: Value = serde_json::from_slice(&out.stdout).unwrap();
    let streams = info["streams"].as_array().unwrap();
    assert!(streams.iter().any(|s| s["codec_type"] == "video"));
    assert!(streams.iter().any(|s| s["codec_type"] == "audio"));
    let duration: f64 = info["format"]["duration"]
        .as_str()
        .expect("durée")
        .parse()
        .unwrap();
    assert!(duration > 1.0, "durée inattendue : {duration} s");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn sigint_finalizes_mp4_recording() {
    run_stop_test(Stop::Signal("INT")).await;
}

#[tokio::test]
async fn sigterm_finalizes_mp4_recording() {
    run_stop_test(Stop::Signal("TERM")).await;
}

#[tokio::test]
async fn stdin_eof_finalizes_mp4_recording() {
    run_stop_test(Stop::StdinEof).await;
}
