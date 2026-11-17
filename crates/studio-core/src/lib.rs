//! Cœur du studio : modèle de scènes, opérations et compositing.

pub mod compositor;
pub mod model;
pub mod studio;

pub use compositor::{Compositor, FrameProvider, NoFrames, VideoFrame};
pub use model::{Canvas, Scene, Source, SourceKind, Transform, MAX_VOLUME};
pub use studio::{Change, SourcePatch, Studio, StudioError};
