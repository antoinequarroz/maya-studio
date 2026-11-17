//! Shell desktop de Maya Studio.
//!
//! Le shell ne fait que trois choses :
//! 1. lancer le moteur `studiod` en sidecar sur 127.0.0.1, port libre et jeton aléatoires ;
//! 2. donner l'URL et le jeton au front-end par la commande `studio_connection` ;
//! 3. arrêter le moteur à la fermeture de l'app.
//!
//! Tout le reste passe par l'API WebSocket du studio (paquet `@suite-media/client`),
//! si bien que le même front-end peut tourner dans un navigateur face à un studio distant.
//!
//! Variables d'environnement utiles en développement :
//! - `MAYA_STUDIO_URL` (+ `MAYA_STUDIO_TOKEN`) : se brancher sur un studio déjà lancé, sans sidecar ;
//! - `MAYA_STUDIO_RECORDINGS` : dossier des enregistrements.

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{Manager, RunEvent};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

/// Ce dont le front-end a besoin pour se connecter au moteur.
#[derive(Debug, Clone, Serialize)]
pub struct StudioConnection {
    pub url: String,
    pub token: Option<String>,
    /// Vrai si le moteur a été lancé par l'app (sidecar), faux pour un studio externe.
    pub managed: bool,
}

struct StudioProcess {
    connection: StudioConnection,
    child: Mutex<Option<CommandChild>>,
}

/// Donne au front-end l'URL WebSocket et le jeton du moteur.
#[tauri::command]
fn studio_connection(state: tauri::State<'_, StudioProcess>) -> StudioConnection {
    state.connection.clone()
}

/// Port TCP libre sur la boucle locale. Le port est relâché juste avant le lancement
/// du moteur ; la fenêtre de course est négligeable sur un poste de régie.
fn free_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// Jeton aléatoire de 128 bits, en hexadécimal.
fn random_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("générateur aléatoire du système indisponible");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn recordings_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(dir) = std::env::var("MAYA_STUDIO_RECORDINGS") {
        return PathBuf::from(dir);
    }
    app.path()
        .video_dir()
        .map(|d| d.join("Maya Studio"))
        .or_else(|_| app.path().app_data_dir().map(|d| d.join("recordings")))
        .unwrap_or_else(|_| PathBuf::from("recordings"))
}

/// Lance `studiod` en sidecar et renvoie l'état à partager.
fn spawn_studiod(app: &tauri::AppHandle) -> Result<StudioProcess, Box<dyn std::error::Error>> {
    if let Ok(url) = std::env::var("MAYA_STUDIO_URL") {
        log::info!("studio externe : {url} (pas de sidecar)");
        return Ok(StudioProcess {
            connection: StudioConnection {
                url,
                token: std::env::var("MAYA_STUDIO_TOKEN").ok(),
                managed: false,
            },
            child: Mutex::new(None),
        });
    }

    let port = free_port()?;
    let token = random_token();
    let listen = format!("127.0.0.1:{port}");
    let recordings = recordings_dir(app);
    std::fs::create_dir_all(&recordings)?;

    let (mut rx, child) = app
        .shell()
        .sidecar("studiod")?
        .args([
            "--listen",
            &listen,
            "--recordings",
            &recordings.to_string_lossy(),
        ])
        // Le jeton passe par l'environnement plutôt que par la ligne de commande,
        // pour ne pas apparaître dans la liste des processus (studiod lit STUDIO_TOKEN).
        .env("STUDIO_TOKEN", &token)
        // Journaux sans codes couleur ANSI : ils sont relayés dans le journal de l'app.
        .env("NO_COLOR", "1")
        .spawn()?;
    log::info!(
        "moteur studiod lancé (pid {}) sur ws://{listen}",
        child.pid()
    );

    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(line) | CommandEvent::Stderr(line) => {
                    log::info!("[studiod] {}", String::from_utf8_lossy(&line).trim_end());
                }
                CommandEvent::Error(e) => log::error!("[studiod] erreur : {e}"),
                CommandEvent::Terminated(status) => {
                    log::warn!(
                        "[studiod] arrêté (code {:?}, signal {:?})",
                        status.code,
                        status.signal
                    );
                }
                _ => {}
            }
        }
    });

    Ok(StudioProcess {
        connection: StudioConnection {
            url: format!("ws://{listen}"),
            token: Some(token),
            managed: true,
        },
        child: Mutex::new(Some(child)),
    })
}

fn stop_studiod(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<StudioProcess>() {
        if let Some(child) = state.child.lock().unwrap().take() {
            log::info!("arrêt du moteur studiod");
            if let Err(e) = child.kill() {
                log::warn!("impossible d'arrêter studiod : {e}");
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let process = spawn_studiod(app.handle())
                .map_err(|e| format!("impossible de lancer le moteur studiod : {e}"))?;
            app.manage(process);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![studio_connection])
        .build(tauri::generate_context!())
        .expect("erreur au démarrage de Maya Studio");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            stop_studiod(handle);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_random_hex() {
        let a = random_token();
        let b = random_token();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn free_port_is_usable() {
        let port = free_port().unwrap();
        assert!(port > 0);
        TcpListener::bind(("127.0.0.1", port)).unwrap();
    }
}
