//! Sorties du studio : enregistrement fichier et diffusion RTMP/SRT.
//!
//! Backend actuel : un processus `ffmpeg` qui reçoit les images RGBA brutes sur
//! son entrée standard. C'est le moyen le plus court d'avoir un bout-en-bout
//! fonctionnel. Le backend cible est GStreamer (pipeline in-process, encodeurs
//! matériels NVENC / QuickSync / VideoToolbox), derrière le même trait.

use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};

use serde::Serialize;
use studio_core::Canvas;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Record,
    Stream,
}

/// Une sortie qui consomme les images programme.
pub trait Output: Send {
    /// Pousse une image RGBA. Une erreur arrête la sortie.
    fn push_frame(&mut self, rgba: &[u8]) -> std::io::Result<()>;
    /// Arrête proprement (finalise le fichier, ferme la connexion).
    fn stop(self: Box<Self>);
    /// Destination : chemin du fichier ou URL de diffusion.
    fn target(&self) -> &str;
}

pub struct FfmpegOutput {
    child: Child,
    stdin: Option<ChildStdin>,
    target: String,
}

impl FfmpegOutput {
    pub fn start(kind: OutputKind, canvas: Canvas, target: &str) -> std::io::Result<Self> {
        let mut cmd =
            Command::new(std::env::var("STUDIO_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()));
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", canvas.width, canvas.height)])
            .args(["-r", &canvas.fps.to_string()])
            .args(["-i", "-"])
            .args([
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-tune",
                "zerolatency",
                "-pix_fmt",
                "yuv420p",
            ])
            .args(["-g", &(canvas.fps * 2).to_string()]);
        match kind {
            OutputKind::Record => {
                cmd.args(["-movflags", "+faststart"]).arg(target);
            }
            OutputKind::Stream => {
                cmd.args(["-b:v", "4500k", "-maxrate", "4500k", "-bufsize", "9000k"]);
                if target.starts_with("srt://") {
                    cmd.args(["-f", "mpegts"]);
                } else {
                    cmd.args(["-f", "flv"]);
                }
                cmd.arg(target);
            }
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take();
        Ok(Self {
            child,
            stdin,
            target: target.to_string(),
        })
    }
}

impl Output for FfmpegOutput {
    fn push_frame(&mut self, rgba: &[u8]) -> std::io::Result<()> {
        match self.stdin.as_mut() {
            Some(s) => s.write_all(rgba),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "sortie fermée",
            )),
        }
    }

    fn stop(mut self: Box<Self>) {
        // Fermer stdin signale la fin du flux : ffmpeg finalise le fichier.
        drop(self.stdin.take());
        let _ = self.child.wait();
    }

    fn target(&self) -> &str {
        &self.target
    }
}
