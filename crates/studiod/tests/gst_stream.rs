//! Diffusion SRT sans récepteur : l'échec doit remonter (OutputFailed), au lieu
//! d'une sortie « active » qui n'envoie rien pendant que srtsink tente de se reconnecter.
#![cfg(feature = "gstreamer")]

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use studio_core::{Canvas, SourceKind, Studio, Transform};
use studiod::engine::{Engine, OutputConfig};
use studiod::outputs::EncoderChoice;
use suite_protocol::ServerMessage;

#[test]
fn srt_without_listener_fails() {
    let canvas = Canvas {
        width: 320,
        height: 240,
        fps: 30,
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
        encoder: EncoderChoice::Named(studiod::encoders::test_encoder().into()),
        ..OutputConfig::default()
    };
    let engine = Engine::new(studio, std::env::temp_dir(), config);
    let mut events = engine.subscribe();
    engine.spawn_render_loop();

    // Port UDP libre, personne n'y écoute.
    let port = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("srt://127.0.0.1:{port}");
    let started = studiod::api::handle(&engine, "StartStream", json!({ "url": url }));
    let failed = match started {
        // Selon la version de srtsink, le refus peut arriver dès le démarrage.
        Err(_) => true,
        Ok(_) => {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut failed = false;
            while Instant::now() < deadline && !failed {
                match events.try_recv() {
                    Ok(ServerMessage::Event {
                        event_type, data, ..
                    }) if event_type == "OutputFailed" => {
                        assert_eq!(data["output"], "stream");
                        failed = true;
                    }
                    Ok(_) => {}
                    Err(_) => std::thread::sleep(Duration::from_millis(50)),
                }
            }
            failed
        }
    };
    assert!(
        failed,
        "OutputFailed attendu pour une connexion SRT refusée"
    );
    let status = studiod::api::handle(&engine, "GetOutputStatus", Value::Null).unwrap();
    assert_eq!(status["stream"]["active"], false);
}
