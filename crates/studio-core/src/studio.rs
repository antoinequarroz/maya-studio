//! État global du studio et opérations qui le modifient.
//!
//! Chaque opération qui change l'état renvoie un [`Change`], que la couche
//! réseau diffuse ensuite comme événement à tous les clients (Nino, UI…).

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::model::{Canvas, Scene, Source, SourceKind, Transform, MAX_VOLUME};

#[derive(Debug, Error, PartialEq)]
pub enum StudioError {
    #[error("scène introuvable : {0}")]
    SceneNotFound(String),
    #[error("source introuvable : {0}")]
    SourceNotFound(String),
    #[error("une scène nommée « {0} » existe déjà")]
    SceneNameTaken(String),
    #[error("impossible de supprimer la scène à l'antenne")]
    SceneOnAir,
    #[error("impossible de supprimer la dernière scène")]
    LastScene,
    #[error("volume invalide : {0} (attendu entre 0 et 2)")]
    InvalidVolume(f32),
}

/// Changement d'état, diffusé comme événement aux clients.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum Change {
    SceneCreated {
        scene: Scene,
    },
    SceneRemoved {
        scene_id: Uuid,
    },
    SceneRenamed {
        scene_id: Uuid,
        name: String,
    },
    ProgramSceneChanged {
        scene_id: Uuid,
    },
    PreviewSceneChanged {
        scene_id: Uuid,
    },
    SourceAdded {
        scene_id: Uuid,
        source: Source,
    },
    SourceRemoved {
        scene_id: Uuid,
        source_id: Uuid,
    },
    SourceUpdated {
        scene_id: Uuid,
        source: Source,
    },
    SourceReordered {
        scene_id: Uuid,
        source_ids: Vec<Uuid>,
    },
    SourceVolumeChanged {
        scene_id: Uuid,
        source_id: Uuid,
        volume: f32,
    },
    SourceMuteChanged {
        scene_id: Uuid,
        source_id: Uuid,
        muted: bool,
    },
}

/// Modification partielle d'une source : seuls les champs présents changent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourcePatch {
    pub name: Option<String>,
    pub visible: Option<bool>,
    pub transform: Option<Transform>,
    /// Gain audio linéaire (0 à 2).
    #[serde(default)]
    pub volume: Option<f32>,
    #[serde(default)]
    pub muted: Option<bool>,
    #[serde(default, flatten)]
    pub kind: Option<SourceKind>,
}

fn check_volume(volume: f32) -> Result<(), StudioError> {
    if volume.is_finite() && (0.0..=MAX_VOLUME).contains(&volume) {
        Ok(())
    } else {
        Err(StudioError::InvalidVolume(volume))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Studio {
    pub canvas: Canvas,
    pub scenes: Vec<Scene>,
    pub program_scene: Uuid,
    pub preview_scene: Uuid,
}

impl Default for Studio {
    fn default() -> Self {
        Self::new(Canvas::default())
    }
}

impl Studio {
    /// Studio neuf avec une scène « Scène 1 » à l'antenne.
    pub fn new(canvas: Canvas) -> Self {
        let scene = Scene::new("Scène 1");
        let id = scene.id;
        Self {
            canvas,
            scenes: vec![scene],
            program_scene: id,
            preview_scene: id,
        }
    }

    /// Trouve une scène par identifiant ou par nom (pratique pour Nino et les scripts).
    pub fn find_scene(&self, key: &str) -> Result<&Scene, StudioError> {
        self.scenes
            .iter()
            .find(|s| s.id.to_string() == key || s.name == key)
            .ok_or_else(|| StudioError::SceneNotFound(key.to_string()))
    }

    fn scene_index(&self, key: &str) -> Result<usize, StudioError> {
        let id = self.find_scene(key)?.id;
        Ok(self
            .scenes
            .iter()
            .position(|s| s.id == id)
            .expect("scène présente"))
    }

    pub fn program(&self) -> &Scene {
        self.scenes
            .iter()
            .find(|s| s.id == self.program_scene)
            .expect("scène programme présente")
    }

    pub fn create_scene(&mut self, name: &str) -> Result<Change, StudioError> {
        if self.scenes.iter().any(|s| s.name == name) {
            return Err(StudioError::SceneNameTaken(name.to_string()));
        }
        let scene = Scene::new(name);
        self.scenes.push(scene.clone());
        Ok(Change::SceneCreated { scene })
    }

    pub fn remove_scene(&mut self, key: &str) -> Result<Change, StudioError> {
        let idx = self.scene_index(key)?;
        let id = self.scenes[idx].id;
        if id == self.program_scene {
            return Err(StudioError::SceneOnAir);
        }
        if self.scenes.len() == 1 {
            return Err(StudioError::LastScene);
        }
        self.scenes.remove(idx);
        if self.preview_scene == id {
            self.preview_scene = self.program_scene;
        }
        Ok(Change::SceneRemoved { scene_id: id })
    }

    pub fn rename_scene(&mut self, key: &str, name: &str) -> Result<Change, StudioError> {
        if self.scenes.iter().any(|s| s.name == name) {
            return Err(StudioError::SceneNameTaken(name.to_string()));
        }
        let idx = self.scene_index(key)?;
        self.scenes[idx].name = name.to_string();
        Ok(Change::SceneRenamed {
            scene_id: self.scenes[idx].id,
            name: name.to_string(),
        })
    }

    pub fn set_program_scene(&mut self, key: &str) -> Result<Change, StudioError> {
        let id = self.find_scene(key)?.id;
        self.program_scene = id;
        Ok(Change::ProgramSceneChanged { scene_id: id })
    }

    pub fn set_preview_scene(&mut self, key: &str) -> Result<Change, StudioError> {
        let id = self.find_scene(key)?.id;
        self.preview_scene = id;
        Ok(Change::PreviewSceneChanged { scene_id: id })
    }

    /// Passe la scène d'aperçu à l'antenne (coupe franche) et inverse les deux,
    /// comme en mode studio.
    pub fn cut(&mut self) -> Vec<Change> {
        std::mem::swap(&mut self.program_scene, &mut self.preview_scene);
        vec![
            Change::ProgramSceneChanged {
                scene_id: self.program_scene,
            },
            Change::PreviewSceneChanged {
                scene_id: self.preview_scene,
            },
        ]
    }

    pub fn add_source(
        &mut self,
        scene_key: &str,
        name: &str,
        kind: SourceKind,
        transform: Option<Transform>,
    ) -> Result<Change, StudioError> {
        let canvas = self.canvas;
        let idx = self.scene_index(scene_key)?;
        let source = Source {
            id: Uuid::new_v4(),
            name: name.to_string(),
            kind,
            transform: transform.unwrap_or_else(|| Transform::full(canvas)),
            visible: true,
            volume: 1.0,
            muted: false,
        };
        self.scenes[idx].sources.push(source.clone());
        Ok(Change::SourceAdded {
            scene_id: self.scenes[idx].id,
            source,
        })
    }

    fn source_index(&self, scene_idx: usize, key: &str) -> Result<usize, StudioError> {
        self.scenes[scene_idx]
            .sources
            .iter()
            .position(|s| s.id.to_string() == key || s.name == key)
            .ok_or_else(|| StudioError::SourceNotFound(key.to_string()))
    }

    pub fn remove_source(
        &mut self,
        scene_key: &str,
        source_key: &str,
    ) -> Result<Change, StudioError> {
        let si = self.scene_index(scene_key)?;
        let idx = self.source_index(si, source_key)?;
        let removed = self.scenes[si].sources.remove(idx);
        Ok(Change::SourceRemoved {
            scene_id: self.scenes[si].id,
            source_id: removed.id,
        })
    }

    pub fn update_source(
        &mut self,
        scene_key: &str,
        source_key: &str,
        patch: SourcePatch,
    ) -> Result<Change, StudioError> {
        if let Some(v) = patch.volume {
            check_volume(v)?;
        }
        let si = self.scene_index(scene_key)?;
        let idx = self.source_index(si, source_key)?;
        let scene_id = self.scenes[si].id;
        let source = &mut self.scenes[si].sources[idx];
        if let Some(v) = patch.volume {
            source.volume = v;
        }
        if let Some(m) = patch.muted {
            source.muted = m;
        }
        if let Some(name) = patch.name {
            source.name = name;
        }
        if let Some(visible) = patch.visible {
            source.visible = visible;
        }
        if let Some(t) = patch.transform {
            source.transform = t;
        }
        if let Some(kind) = patch.kind {
            source.kind = kind;
        }
        Ok(Change::SourceUpdated {
            scene_id,
            source: source.clone(),
        })
    }

    /// Règle le gain audio d'une source (linéaire, 0 à 2).
    pub fn set_source_volume(
        &mut self,
        scene_key: &str,
        source_key: &str,
        volume: f32,
    ) -> Result<Change, StudioError> {
        check_volume(volume)?;
        let si = self.scene_index(scene_key)?;
        let idx = self.source_index(si, source_key)?;
        let scene_id = self.scenes[si].id;
        let source = &mut self.scenes[si].sources[idx];
        source.volume = volume;
        Ok(Change::SourceVolumeChanged {
            scene_id,
            source_id: source.id,
            volume,
        })
    }

    /// Coupe ou rétablit le son d'une source.
    pub fn set_source_muted(
        &mut self,
        scene_key: &str,
        source_key: &str,
        muted: bool,
    ) -> Result<Change, StudioError> {
        let si = self.scene_index(scene_key)?;
        let idx = self.source_index(si, source_key)?;
        let scene_id = self.scenes[si].id;
        let source = &mut self.scenes[si].sources[idx];
        source.muted = muted;
        Ok(Change::SourceMuteChanged {
            scene_id,
            source_id: source.id,
            muted,
        })
    }

    /// Toutes les sources de capture, toutes scènes confondues.
    pub fn capture_sources(&self) -> impl Iterator<Item = &Source> {
        self.scenes
            .iter()
            .flat_map(|s| s.sources.iter())
            .filter(|s| s.kind.is_capture())
    }

    /// Déplace une source dans la pile (0 = tout en dessous).
    pub fn move_source(
        &mut self,
        scene_key: &str,
        source_key: &str,
        index: usize,
    ) -> Result<Change, StudioError> {
        let si = self.scene_index(scene_key)?;
        let idx = self.source_index(si, source_key)?;
        let sources = &mut self.scenes[si].sources;
        let s = sources.remove(idx);
        let index = index.min(sources.len());
        sources.insert(index, s);
        Ok(Change::SourceReordered {
            scene_id: self.scenes[si].id,
            source_ids: self.scenes[si].sources.iter().map(|s| s.id).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_switch_and_cut() {
        let mut s = Studio::default();
        s.create_scene("Plateau").unwrap();
        s.set_preview_scene("Plateau").unwrap();
        s.cut();
        assert_eq!(s.program().name, "Plateau");
        assert_eq!(
            s.find_scene(&s.preview_scene.to_string()).unwrap().name,
            "Scène 1"
        );
    }

    #[test]
    fn cannot_remove_on_air_scene() {
        let mut s = Studio::default();
        assert_eq!(s.remove_scene("Scène 1"), Err(StudioError::SceneOnAir));
    }

    #[test]
    fn duplicate_scene_name_rejected() {
        let mut s = Studio::default();
        assert!(matches!(
            s.create_scene("Scène 1"),
            Err(StudioError::SceneNameTaken(_))
        ));
    }

    #[test]
    fn sources_add_update_move_remove() {
        let mut s = Studio::default();
        s.add_source(
            "Scène 1",
            "Fond",
            SourceKind::Color {
                rgba: [0, 0, 255, 255],
            },
            None,
        )
        .unwrap();
        s.add_source("Scène 1", "Mire", SourceKind::TestPattern, None)
            .unwrap();
        s.update_source(
            "Scène 1",
            "Mire",
            SourcePatch {
                visible: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!s.program().sources[1].visible);
        s.move_source("Scène 1", "Mire", 0).unwrap();
        assert_eq!(s.program().sources[0].name, "Mire");
        s.remove_source("Scène 1", "Fond").unwrap();
        assert_eq!(s.program().sources.len(), 1);
    }

    #[test]
    fn volume_and_mute() {
        let mut s = Studio::default();
        s.add_source(
            "Scène 1",
            "Micro",
            SourceKind::AudioInput { device_id: None },
            None,
        )
        .unwrap();
        assert_eq!(s.program().sources[0].volume, 1.0);
        assert!(!s.program().sources[0].muted);
        s.set_source_volume("Scène 1", "Micro", 1.5).unwrap();
        s.set_source_muted("Scène 1", "Micro", true).unwrap();
        assert_eq!(s.program().sources[0].volume, 1.5);
        assert!(s.program().sources[0].muted);
        assert!(matches!(
            s.set_source_volume("Scène 1", "Micro", 2.5),
            Err(StudioError::InvalidVolume(_))
        ));
        assert!(s.set_source_volume("Scène 1", "Micro", f32::NAN).is_err());
        assert!(s.set_source_volume("Scène 1", "Micro", -0.1).is_err());
        assert_eq!(s.capture_sources().count(), 1);
    }

    #[test]
    fn new_kinds_serde() {
        use serde_json::json;
        let src: Source = serde_json::from_value(json!({
            "id": Uuid::nil(), "name": "Clip", "kind": "media_file",
            "path": "a.mp4", "loop": true,
            "transform": {"x": 0, "y": 0, "width": 10, "height": 10},
            "visible": true
        }))
        .unwrap();
        assert_eq!(
            src.kind,
            SourceKind::MediaFile {
                path: "a.mp4".into(),
                looping: true
            }
        );
        // Anciennes sources sans volume ni mute : valeurs par défaut.
        assert_eq!(src.volume, 1.0);
        assert!(!src.muted);
        let v = serde_json::to_value(&src).unwrap();
        assert_eq!(v["loop"], true);
        assert_eq!(v["kind"], "media_file");
        let cam: SourceKind = serde_json::from_value(json!({"kind": "camera"})).unwrap();
        assert_eq!(
            cam,
            SourceKind::Camera {
                device_id: None,
                width: None,
                height: None,
                fps: None
            }
        );
        let cam: SourceKind =
            serde_json::from_value(json!({"kind": "camera", "device_id": "x", "fps": 60})).unwrap();
        assert!(matches!(cam, SourceKind::Camera { fps: Some(60), .. }));
        for k in [
            json!({"kind": "display_capture", "display_id": "0"}),
            json!({"kind": "window_capture"}),
            json!({"kind": "audio_input"}),
            json!({"kind": "srt_input", "url": "srt://h:1"}),
            json!({"kind": "test_pattern"}),
        ] {
            let kind: SourceKind = serde_json::from_value(k.clone()).unwrap();
            assert_eq!(serde_json::to_value(&kind).unwrap(), k);
        }
    }
}
