//! Serveur WebSocket : poignée de main, authentification, requêtes et événements.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use suite_protocol::{AppInfo, ClientMessage, ServerMessage, PROTOCOL_VERSION};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::api;
use crate::engine::Engine;

static NEXT_CLIENT: AtomicU64 = AtomicU64::new(1);

pub async fn serve(engine: Arc<Engine>, addr: &str, token: Option<String>) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("API WebSocket à l'écoute sur ws://{addr}");
    loop {
        let (stream, peer) = listener.accept().await?;
        let engine = Arc::clone(&engine);
        let token = token.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(engine, stream, token).await {
                tracing::debug!(%peer, error = %e, "client déconnecté");
            }
        });
    }
}

fn to_msg(m: &ServerMessage) -> Message {
    Message::Text(
        serde_json::to_string(m)
            .expect("message sérialisable")
            .into(),
    )
}

async fn handle_client(
    engine: Arc<Engine>,
    stream: TcpStream,
    token: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ws = tokio_tungstenite::accept_async(stream).await?;
    let (mut sink, mut source) = ws.split();
    let client_id = NEXT_CLIENT.fetch_add(1, Ordering::Relaxed).to_string();

    sink.send(to_msg(&ServerMessage::Hello {
        app: AppInfo {
            id: "studio".into(),
            name: "Maya Studio".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        protocol: PROTOCOL_VERSION,
        auth_required: token.is_some(),
        capabilities: api::CAPABILITIES.iter().map(|c| c.to_string()).collect(),
    }))
    .await?;

    // Toutes les écritures passent par une tâche unique pour ne pas entrelacer.
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(to_msg(&m)).await.is_err() {
                break;
            }
        }
    });

    let mut identified = false;
    let mut event_task: Option<tokio::task::JoinHandle<()>> = None;

    while let Some(msg) = source.next().await {
        let text = match msg? {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let parsed: ClientMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                let _ = tx.send(ServerMessage::err(
                    "",
                    "invalid_message",
                    format!("message invalide : {e}"),
                ));
                continue;
            }
        };
        match parsed {
            ClientMessage::Identify {
                token: given,
                name,
                app,
                events,
            } => {
                if token.is_some() && given != token {
                    let _ = tx.send(ServerMessage::err("", "auth_failed", "jeton invalide"));
                    break;
                }
                identified = true;
                let app_id = app.as_ref().map(|a| a.id.as_str()).unwrap_or("-");
                tracing::info!(
                    client_id,
                    name = name.as_deref().unwrap_or("?"),
                    app = app_id,
                    "client identifié"
                );
                if events && event_task.is_none() {
                    let mut sub = engine.subscribe();
                    let tx = tx.clone();
                    event_task = Some(tokio::spawn(async move {
                        loop {
                            match sub.recv().await {
                                Ok(ev) => {
                                    if tx.send(ev).is_err() {
                                        break;
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    continue
                                }
                                Err(_) => break,
                            }
                        }
                    }));
                }
                let _ = tx.send(ServerMessage::Identified {
                    client_id: client_id.clone(),
                });
            }
            ClientMessage::Request {
                id,
                request_type,
                data,
            } => {
                if !identified {
                    let _ = tx.send(ServerMessage::err(
                        id,
                        "not_identified",
                        "envoie d'abord « identify »",
                    ));
                    continue;
                }
                // Les requêtes de sortie peuvent bloquer (lancement de processus) :
                // on les exécute hors de la boucle async.
                let engine = Arc::clone(&engine);
                let reply = tokio::task::spawn_blocking(move || {
                    match api::handle(&engine, &request_type, data) {
                        Ok(v) => ServerMessage::ok(id, v),
                        Err((code, message)) => ServerMessage::err(id, code, message),
                    }
                })
                .await?;
                let _ = tx.send(reply);
            }
        }
    }

    if let Some(t) = event_task {
        t.abort();
    }
    drop(tx);
    let _ = writer.await;
    Ok(())
}
