//! `studiod` : moteur du studio, sans interface, piloté par l'API WebSocket.

use clap::Parser;
use studio_core::{Canvas, SourceKind, Studio, Transform};
use studiod::engine::{self, OutputConfig};
use studiod::outputs::{
    Backend, EncoderChoice, DEFAULT_RECORD_BITRATE_KBPS, DEFAULT_STREAM_BITRATE_KBPS,
};
use studiod::server;

#[derive(Parser)]
#[command(
    name = "studiod",
    version,
    about = "Moteur de Maya Studio, piloté en WebSocket"
)]
struct Args {
    /// Adresse d'écoute de l'API WebSocket.
    #[arg(long, default_value = "127.0.0.1:4460")]
    listen: String,
    /// Jeton exigé à l'identification (aussi via STUDIO_TOKEN).
    #[arg(long, env = "STUDIO_TOKEN")]
    token: Option<String>,
    /// Dossier des enregistrements.
    #[arg(long, default_value = "recordings")]
    recordings: std::path::PathBuf,
    #[arg(long, default_value_t = 1280)]
    width: u32,
    #[arg(long, default_value_t = 720)]
    height: u32,
    #[arg(long, default_value_t = 30)]
    fps: u32,
    /// Backend des sorties : `gstreamer` (in-process, par défaut) ou `ffmpeg`
    /// (processus externe avec libx264, GPL, développement seulement).
    #[arg(long, default_value = "gstreamer")]
    output_backend: Backend,
    /// Encodeur H.264 : `auto` (matériel si présent, sinon openh264enc) ou un nom
    /// d'élément GStreamer (`nvh264enc`, `vtenc_h264_hw`…). `x264enc` (GPL) uniquement ici.
    #[arg(long, default_value = "auto")]
    encoder: EncoderChoice,
    /// Débit vidéo de l'enregistrement, en kbit/s.
    #[arg(long, default_value_t = DEFAULT_RECORD_BITRATE_KBPS)]
    record_bitrate: u32,
    /// Débit vidéo de la diffusion, en kbit/s.
    #[arg(long, default_value_t = DEFAULT_STREAM_BITRATE_KBPS)]
    stream_bitrate: u32,
    /// S'arrêter quand l'entrée standard se ferme. À utiliser quand studiod est lancé
    /// par une autre app (sidecar) qui garde le côté écriture du tube : si cette app
    /// meurt, même brutalement, le système ferme le tube et le moteur s'arrête aussi.
    #[arg(long)]
    exit_on_stdin_eof: bool,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();

    let canvas = Canvas {
        width: args.width,
        height: args.height,
        fps: args.fps,
    };
    let mut studio = Studio::new(canvas);
    // Scène de démarrage : une mire, pour voir tout de suite que la sortie vit.
    studio
        .add_source(
            "Scène 1",
            "Mire",
            SourceKind::TestPattern,
            Some(Transform::full(canvas)),
        )
        .expect("scène par défaut");

    let output_config = OutputConfig {
        backend: args.output_backend,
        encoder: args.encoder,
        record_bitrate_kbps: args.record_bitrate,
        stream_bitrate_kbps: args.stream_bitrate,
    };
    check_backend(&output_config)?;

    let engine = engine::Engine::new(studio, args.recordings, output_config);
    engine.spawn_render_loop();
    // Avec --exit-on-stdin-eof, la fermeture de l'entrée standard (app parente partie)
    // suit le même chemin d'arrêt propre qu'un signal.
    let parent_gone = async {
        if args.exit_on_stdin_eof {
            watch_parent().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    let result = tokio::select! {
        r = server::serve(std::sync::Arc::clone(&engine), &args.listen, args.token) => r,
        signal = shutdown_signal() => {
            tracing::info!(%signal, "arrêt demandé, finalisation des sorties");
            Ok(())
        }
        _ = parent_gone => {
            tracing::warn!("entrée standard fermée : l'app parente est partie, arrêt du moteur");
            Ok(())
        }
    };
    // Sans cela, un MP4 non fragmenté en cours resterait illisible (atome moov absent).
    let stopped = tokio::task::spawn_blocking(move || {
        let stopped = engine.shutdown_outputs();
        engine.shutdown_captures();
        stopped
    })
    .await
    .unwrap_or_default();
    for data in &stopped {
        tracing::info!(
            output = %data["output"],
            finalized = %data["finalized"],
            "sortie arrêtée à la fermeture"
        );
    }
    result
}

/// Attend SIGINT (Ctrl-C) ou, sous Unix, SIGTERM ; renvoie le nom du signal reçu.
async fn shutdown_signal() -> &'static str {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "écoute de Ctrl-C impossible");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => tokio::select! {
                _ = ctrl_c => "SIGINT",
                _ = term.recv() => "SIGTERM",
            },
            Err(e) => {
                tracing::error!(error = %e, "écoute de SIGTERM impossible");
                ctrl_c.await;
                "SIGINT"
            }
        }
    }
    #[cfg(not(unix))]
    {
        ctrl_c.await;
        "Ctrl-C"
    }
}

/// Vérifie dès le lancement que le backend et l'encodeur demandés sont utilisables.
fn check_backend(cfg: &OutputConfig) -> std::io::Result<()> {
    match cfg.backend {
        Backend::Ffmpeg => {
            tracing::warn!(
                "backend de sortie ffmpeg (provisoire, libx264 sous GPL) : développement seulement"
            );
            Ok(())
        }
        #[cfg(feature = "gstreamer")]
        Backend::Gstreamer => {
            gstreamer::init().map_err(|e| {
                std::io::Error::other(format!("initialisation de GStreamer impossible : {e}"))
            })?;
            match studiod::encoders::select(&cfg.encoder) {
                Ok(name) => tracing::info!(
                    encoder = %name,
                    hardware = studiod::encoders::is_hardware(&name),
                    "backend de sortie GStreamer"
                ),
                // Pas fatal : l'erreur sera renvoyée au démarrage d'une sortie.
                Err(e) => tracing::error!(error = %e, "aucun encodeur H.264 utilisable"),
            }
            Ok(())
        }
        #[cfg(not(feature = "gstreamer"))]
        Backend::Gstreamer => Err(std::io::Error::other(
            "studiod a été compilé sans la fonctionnalité « gstreamer » : utilise --output-backend ffmpeg",
        )),
    }
}

/// Attend la fin de l'entrée standard, lue dans un thread dédié (la lecture de stdin
/// est bloquante). Le chemin d'arrêt propre (sorties finalisées, captures arrêtées) est
/// ensuite celui de `main`, comme pour SIGINT / SIGTERM.
async fn watch_parent() {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let spawned = std::thread::Builder::new()
        .name("parent-watch".into())
        .spawn(move || {
            // On lit et on jette : seul compte le moment où le tube se ferme.
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
            let _ = tx.send(());
        });
    match spawned {
        Ok(_) => {
            let _ = rx.await;
        }
        Err(e) => {
            tracing::error!(error = %e, "surveillance de l'app parente impossible");
            std::future::pending::<()>().await;
        }
    }
}
