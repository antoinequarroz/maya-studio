//! `studiod` : moteur du studio, sans interface, piloté par l'API WebSocket.

mod api;
mod engine;
mod outputs;
mod server;

use clap::Parser;
use studio_core::{Canvas, SourceKind, Studio, Transform};

#[derive(Parser)]
#[command(
    name = "studiod",
    version,
    about = "Moteur du studio media, piloté en WebSocket"
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

    let engine = engine::Engine::new(studio, args.recordings);
    engine.spawn_render_loop();
    server::serve(engine, &args.listen, args.token).await
}
