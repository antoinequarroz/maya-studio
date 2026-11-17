//! Arrêt de `studiod` par signal : l'enregistrement MP4 (non fragmenté, le format
//! par défaut) doit être finalisé, sinon le fichier est illisible (atome moov absent).
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

async fn run_signal_test(signal: &str) {
    let dir =
        std::env::temp_dir().join(format!("studiod-signal-{}-{}", std::process::id(), signal));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let addr = format!("127.0.0.1:{}", free_port());
    let mut child = Command::new(env!("CARGO_BIN_EXE_studiod"))
        .args(["--listen", &addr, "--width", "320", "--height", "240"])
        .args(["--encoder", "openh264enc", "--recordings"])
        .arg(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("lancement de studiod");

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

    let status = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(15);
    let exit = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("studiod ne s'est pas arrêté après SIG{signal}");
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
    run_signal_test("INT").await;
}

#[tokio::test]
async fn sigterm_finalizes_mp4_recording() {
    run_signal_test("TERM").await;
}
