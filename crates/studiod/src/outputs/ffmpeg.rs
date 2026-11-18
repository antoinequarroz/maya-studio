//! Backend provisoire : un processus `ffmpeg` qui reçoit les images RGBA brutes
//! sur son entrée standard.
//!
//! Réservé au développement (`studiod --output-backend ffmpeg`) : il encode avec
//! `libx264`, sous licence GPL, et son `push_frame` peut bloquer si ffmpeg ne
//! suit pas. La piste audio AAC est un silence généré par ffmpeg lui-même.

use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;

use super::{AudioSink, Output, OutputKind, OutputSettings, StreamProtocol, AUDIO_RATE};

pub struct FfmpegOutput {
    child: Child,
    stdin: Option<ChildStdin>,
    target: String,
}

impl FfmpegOutput {
    pub fn start(settings: &OutputSettings) -> std::io::Result<Self> {
        tracing::warn!(
            "backend ffmpeg : encodage libx264 (GPL), réservé au développement ; \
             utilise --output-backend gstreamer pour une distribution"
        );
        let canvas = settings.canvas;
        let target = settings.target.as_str();
        let kbps = settings.bitrate_kbps;
        let mut cmd =
            Command::new(std::env::var("STUDIO_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()));
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", canvas.width, canvas.height)])
            .args(["-r", &canvas.fps.to_string()])
            .args(["-i", "-"])
            .args(["-f", "lavfi", "-i"])
            .arg(format!("anullsrc=r={AUDIO_RATE}:cl=stereo"))
            .args(["-map", "0:v", "-map", "1:a", "-shortest"])
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
            .args(["-b:v", &format!("{kbps}k")])
            .args(["-maxrate", &format!("{kbps}k")])
            .args(["-bufsize", &format!("{}k", kbps * 2)])
            .args(["-g", &(canvas.fps * 2).to_string()])
            .args([
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-ar",
                &AUDIO_RATE.to_string(),
            ]);
        match settings.kind {
            OutputKind::Record => {
                if settings.fragmented {
                    cmd.args(["-movflags", "+frag_keyframe+empty_moov"]);
                }
                cmd.arg(target);
            }
            OutputKind::Stream => {
                match StreamProtocol::from_url(target) {
                    Some(StreamProtocol::Srt) => cmd.args(["-f", "mpegts"]),
                    Some(StreamProtocol::Rtmp) => cmd.args(["-f", "flv"]),
                    None => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "unsupported_url",
                        ))
                    }
                };
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

    fn audio_sink(&self) -> Option<Arc<dyn AudioSink>> {
        // ffmpeg génère lui-même la piste silencieuse.
        None
    }

    fn stop(mut self: Box<Self>) -> Result<(), String> {
        // Fermer stdin signale la fin du flux : ffmpeg finalise le fichier.
        drop(self.stdin.take());
        match self.child.wait() {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("ffmpeg s'est terminé avec le code {s}")),
            Err(e) => Err(format!("ffmpeg : {e}")),
        }
    }

    fn target(&self) -> &str {
        &self.target
    }

    fn encoder(&self) -> &str {
        "libx264"
    }

    fn dropped_frames(&self) -> u64 {
        0
    }
}
