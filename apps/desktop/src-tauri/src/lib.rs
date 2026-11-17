//! Shell desktop de Maya Studio.
//!
//! Le shell ne fait que trois choses :
//! 1. lancer le moteur `studiod` en sidecar sur 127.0.0.1, port libre et jeton aléatoires ;
//! 2. donner l'URL et le jeton au front-end par la commande `studio_connection` ;
//! 3. arrêter le moteur à la fermeture de l'app, le relancer s'il plante (en prévenant
//!    le front-end par l'événement `studio-engine`), et faire en sorte qu'il s'arrête
//!    aussi si l'app meurt brutalement (`--exit-on-stdin-eof`).
//!
//! Tout le reste passe par l'API WebSocket du studio (paquet `@suite-media/client`),
//! si bien que le même front-end peut tourner dans un navigateur face à un studio distant.
//!
//! Variables d'environnement utiles en développement :
//! - `MAYA_STUDIO_URL` (+ `MAYA_STUDIO_TOKEN`) : se brancher sur un studio déjà lancé, sans sidecar ;
//! - `MAYA_STUDIO_RECORDINGS` : dossier des enregistrements.

use std::collections::VecDeque;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{Emitter, Manager, RunEvent};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

/// Événement émis vers le front-end quand le moteur lancé par l'app s'arrête ou redémarre.
pub const ENGINE_EVENT: &str = "studio-engine";

/// Au-delà de `MAX_RESTARTS` redémarrages en `RESTART_WINDOW`, on abandonne :
/// le moteur plante en boucle et le relancer ne ferait que masquer le problème.
const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(60);
const RESTART_DELAY: Duration = Duration::from_secs(1);

/// Ce dont le front-end a besoin pour se connecter au moteur.
#[derive(Debug, Clone, Serialize)]
pub struct StudioConnection {
    pub url: String,
    pub token: Option<String>,
    /// Vrai si le moteur a été lancé par l'app (sidecar), faux pour un studio externe.
    pub managed: bool,
}

/// Charge utile de [`ENGINE_EVENT`].
#[derive(Debug, Clone, Serialize)]
pub struct EngineEvent {
    /// `stopped` (arrêt inattendu), `restarted` (relancé) ou `failed` (abandon).
    pub kind: &'static str,
    pub message: String,
}

/// Paramètres de lancement du sidecar, réutilisés tels quels à chaque redémarrage
/// pour que le front-end se reconnecte sur la même URL avec le même jeton.
#[derive(Debug, Clone)]
struct SidecarConfig {
    listen: String,
    token: String,
    recordings: PathBuf,
}

/// Partagé entre l'état Tauri et la tâche qui surveille le sidecar.
#[derive(Default)]
struct SidecarShared {
    child: Mutex<Option<CommandChild>>,
    /// Mis à vrai avant l'arrêt volontaire : la fin du processus n'est alors pas une panne.
    shutting_down: AtomicBool,
}

struct StudioProcess {
    connection: StudioConnection,
    sidecar: Arc<SidecarShared>,
}

/// Fenêtre glissante des redémarrages récents.
struct RestartBudget {
    max: usize,
    window: Duration,
    recent: VecDeque<Instant>,
}

impl RestartBudget {
    fn new(max: usize, window: Duration) -> Self {
        Self {
            max,
            window,
            recent: VecDeque::new(),
        }
    }

    /// Vrai si un redémarrage est encore permis à `now` (et le compte).
    fn allow(&mut self, now: Instant) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|t| now.duration_since(*t) > self.window)
        {
            self.recent.pop_front();
        }
        if self.recent.len() >= self.max {
            return false;
        }
        self.recent.push_back(now);
        true
    }
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

/// Lance un processus `studiod`.
///
/// - Groupe de processus (POSIX) ou objet job (Windows) : l'arrêt emporte aussi les
///   processus enfants du moteur (encodeurs externes).
/// - `--exit-on-stdin-eof` : le shell garde le côté écriture de l'entrée standard du
///   moteur ; si le shell meurt brutalement (SIGKILL, plantage, gestionnaire des tâches,
///   recompilation en `tauri dev`), le système ferme le tube et le moteur s'arrête,
///   au lieu de continuer à rendre et à diffuser sur un port que plus personne ne connaît.
fn launch(
    app: &tauri::AppHandle,
    cfg: &SidecarConfig,
) -> Result<(tauri::async_runtime::Receiver<CommandEvent>, CommandChild), Box<dyn std::error::Error>>
{
    let (rx, child) = app
        .shell()
        .sidecar("studiod")?
        .args([
            "--listen",
            &cfg.listen,
            "--recordings",
            &cfg.recordings.to_string_lossy(),
            "--exit-on-stdin-eof",
        ])
        // Le jeton passe par l'environnement plutôt que par la ligne de commande,
        // pour ne pas apparaître dans la liste des processus (studiod lit STUDIO_TOKEN).
        .env("STUDIO_TOKEN", &cfg.token)
        // Journaux sans codes couleur ANSI : ils sont relayés dans le journal de l'app.
        .env("NO_COLOR", "1")
        .set_process_group(true)
        .spawn()?;
    log::info!(
        "moteur studiod lancé (pid {}) sur ws://{}",
        child.pid(),
        cfg.listen
    );
    Ok((rx, child))
}

fn emit_engine(app: &tauri::AppHandle, kind: &'static str, message: String) {
    if let Err(e) = app.emit(ENGINE_EVENT, EngineEvent { kind, message }) {
        log::warn!("événement {ENGINE_EVENT} non transmis : {e}");
    }
}

/// Relaie les journaux du moteur et le relance s'il s'arrête sans qu'on le lui ait demandé.
fn watch_sidecar(
    app: tauri::AppHandle,
    cfg: SidecarConfig,
    shared: Arc<SidecarShared>,
    mut rx: tauri::async_runtime::Receiver<CommandEvent>,
) {
    tauri::async_runtime::spawn(async move {
        let mut budget = RestartBudget::new(MAX_RESTARTS, RESTART_WINDOW);
        loop {
            let mut status_text = String::from("fin inattendue");
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) | CommandEvent::Stderr(line) => {
                        log::info!("[studiod] {}", String::from_utf8_lossy(&line).trim_end());
                    }
                    CommandEvent::Error(e) => log::error!("[studiod] erreur : {e}"),
                    CommandEvent::Terminated(status) => {
                        status_text = match (status.code, status.signal) {
                            (Some(code), _) => format!("code {code}"),
                            (None, Some(sig)) => format!("signal {sig}"),
                            _ => "fin inattendue".into(),
                        };
                        log::warn!("[studiod] arrêté ({status_text})");
                    }
                    _ => {}
                }
            }
            if shared.shutting_down.load(Ordering::SeqCst) {
                return;
            }
            shared.child.lock().unwrap().take();

            if !budget.allow(Instant::now()) {
                let message = format!(
                    "Le moteur du studio s'est arrêté ({status_text}) et plante en boucle : \
                     il n'est plus relancé. Redémarre l'application."
                );
                log::error!("{message}");
                emit_engine(&app, "failed", message);
                return;
            }
            emit_engine(
                &app,
                "stopped",
                format!("Le moteur du studio s'est arrêté ({status_text}). Redémarrage…"),
            );
            tokio::time::sleep(RESTART_DELAY).await;
            if shared.shutting_down.load(Ordering::SeqCst) {
                return;
            }
            match launch(&app, &cfg) {
                Ok((new_rx, child)) => {
                    *shared.child.lock().unwrap() = Some(child);
                    rx = new_rx;
                    emit_engine(
                        &app,
                        "restarted",
                        "Moteur du studio relancé : les sorties en cours ont été interrompues."
                            .into(),
                    );
                }
                Err(e) => {
                    let message = format!("Impossible de relancer le moteur du studio : {e}");
                    log::error!("{message}");
                    emit_engine(&app, "failed", message);
                    return;
                }
            }
        }
    });
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
            sidecar: Arc::default(),
        });
    }

    let cfg = SidecarConfig {
        listen: format!("127.0.0.1:{}", free_port()?),
        token: random_token(),
        recordings: recordings_dir(app),
    };
    std::fs::create_dir_all(&cfg.recordings)?;

    let (rx, child) = launch(app, &cfg)?;
    let sidecar = Arc::new(SidecarShared {
        child: Mutex::new(Some(child)),
        shutting_down: AtomicBool::new(false),
    });
    let connection = StudioConnection {
        url: format!("ws://{}", cfg.listen),
        token: Some(cfg.token.clone()),
        managed: true,
    };
    watch_sidecar(app.clone(), cfg, Arc::clone(&sidecar), rx);
    Ok(StudioProcess {
        connection,
        sidecar,
    })
}

fn stop_studiod(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<StudioProcess>() {
        state.sidecar.shutting_down.store(true, Ordering::SeqCst);
        if let Some(child) = state.sidecar.child.lock().unwrap().take() {
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
    fn restart_budget_limits_crash_loops() {
        let mut budget = RestartBudget::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(budget.allow(t0));
        assert!(budget.allow(t0 + Duration::from_secs(1)));
        assert!(budget.allow(t0 + Duration::from_secs(2)));
        assert!(!budget.allow(t0 + Duration::from_secs(3)));
        // Une fois la fenêtre passée, le moteur peut de nouveau être relancé.
        assert!(budget.allow(t0 + Duration::from_secs(63)));
    }

    #[test]
    fn free_port_is_usable() {
        let port = free_port().unwrap();
        assert!(port > 0);
        TcpListener::bind(("127.0.0.1", port)).unwrap();
    }
}
