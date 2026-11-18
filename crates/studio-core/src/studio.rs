//! État global du studio et opérations qui le modifient.
//!
//! Chaque opération qui change l'état renvoie un [`Change`], que la couche
//! réseau diffuse ensuite comme événement à tous les clients (Nino, UI…).

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::model::{
    AudioInput, Canvas, Scene, Source, SourceKind, Transform, MAX_META_BYTES, MAX_VOLUME,
};

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
    #[error("entrée audio introuvable : {0}")]
    InputNotFound(String),
    #[error("une entrée audio nommée « {0} » existe déjà")]
    InputNameTaken(String),
    #[error("type d'entrée audio non pris en charge : seul audio_input est accepté")]
    InvalidInputKind,
    #[error("métadonnées invalides : {0}")]
    InvalidMeta(String),
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
    AudioInputCreated {
        input: AudioInput,
    },
    AudioInputRemoved {
        input_id: Uuid,
    },
    AudioInputUpdated {
        input: AudioInput,
    },
    AudioInputVolumeChanged {
        input_id: Uuid,
        volume: f32,
    },
    AudioInputMuteChanged {
        input_id: Uuid,
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
    /// Remplace les métadonnées du client (`null` pour les effacer).
    #[serde(default, deserialize_with = "patch_meta")]
    pub meta: Option<Option<serde_json::Value>>,
    #[serde(default, flatten)]
    pub kind: Option<SourceKind>,
}

/// Modification partielle d'une entrée audio globale.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AudioInputPatch {
    pub name: Option<String>,
    /// Remplace les métadonnées du client (`null` pour les effacer).
    #[serde(default, deserialize_with = "patch_meta")]
    pub meta: Option<Option<serde_json::Value>>,
    #[serde(default, flatten)]
    pub kind: Option<SourceKind>,
}

/// Distingue un champ `meta` absent (`None`) d'un `meta: null` (`Some(None)`).
fn patch_meta<'de, D>(d: D) -> Result<Option<Option<serde_json::Value>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(Some(if v.is_null() { None } else { Some(v) }))
}

/// Les métadonnées sont un objet JSON d'au plus [`MAX_META_BYTES`] octets.
pub fn check_meta(meta: &Option<serde_json::Value>) -> Result<(), StudioError> {
    let Some(v) = meta else { return Ok(()) };
    if !v.is_object() {
        return Err(StudioError::InvalidMeta("objet JSON attendu".into()));
    }
    let len = serde_json::to_string(v)
        .map(|s| s.len())
        .unwrap_or(usize::MAX);
    if len > MAX_META_BYTES {
        return Err(StudioError::InvalidMeta(format!(
            "{len} octets, au plus {MAX_META_BYTES}"
        )));
    }
    Ok(())
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
    /// Entrées audio globales (domaine `studio.mixer`), indépendantes des scènes.
    #[serde(default)]
    pub audio_inputs: Vec<AudioInput>,
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
            audio_inputs: Vec::new(),
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
        self.add_source_with_meta(scene_key, name, kind, transform, None)
    }

    /// Comme [`Studio::add_source`], avec les métadonnées du client.
    pub fn add_source_with_meta(
        &mut self,
        scene_key: &str,
        name: &str,
        kind: SourceKind,
        transform: Option<Transform>,
        meta: Option<serde_json::Value>,
    ) -> Result<Change, StudioError> {
        check_meta(&meta)?;
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
            meta,
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
        if let Some(meta) = &patch.meta {
            check_meta(meta)?;
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
        if let Some(meta) = patch.meta {
            source.meta = meta;
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

    /// Toutes les captures à faire tourner : sources de capture des scènes (une
    /// fois chacune) et entrées audio globales.
    pub fn capture_list(&self) -> Vec<(Uuid, SourceKind)> {
        let mut seen = std::collections::HashSet::new();
        self.capture_sources()
            .map(|s| (s.id, s.kind.clone()))
            .chain(self.audio_inputs.iter().map(|i| (i.id, i.kind.clone())))
            .filter(|(id, _)| seen.insert(*id))
            .collect()
    }

    /// Trouve une entrée audio globale par identifiant ou par nom.
    pub fn find_audio_input(&self, key: &str) -> Result<&AudioInput, StudioError> {
        self.audio_inputs
            .iter()
            .find(|i| i.id.to_string() == key || i.name == key)
            .ok_or_else(|| StudioError::InputNotFound(key.to_string()))
    }

    fn audio_input_index(&self, key: &str) -> Result<usize, StudioError> {
        let id = self.find_audio_input(key)?.id;
        Ok(self
            .audio_inputs
            .iter()
            .position(|i| i.id == id)
            .expect("entrée présente"))
    }

    fn check_input_kind(kind: &SourceKind) -> Result<(), StudioError> {
        match kind {
            SourceKind::AudioInput { .. } => Ok(()),
            _ => Err(StudioError::InvalidInputKind),
        }
    }

    /// Ajoute une entrée audio globale. Les noms sont uniques : Nino désigne ses
    /// micros par leur nom.
    pub fn create_audio_input(
        &mut self,
        name: &str,
        kind: SourceKind,
        volume: Option<f32>,
        muted: bool,
        meta: Option<serde_json::Value>,
    ) -> Result<Change, StudioError> {
        Self::check_input_kind(&kind)?;
        if let Some(v) = volume {
            check_volume(v)?;
        }
        check_meta(&meta)?;
        if self.audio_inputs.iter().any(|i| i.name == name) {
            return Err(StudioError::InputNameTaken(name.to_string()));
        }
        let input = AudioInput {
            id: Uuid::new_v4(),
            name: name.to_string(),
            kind,
            volume: volume.unwrap_or(1.0),
            muted,
            meta,
        };
        self.audio_inputs.push(input.clone());
        Ok(Change::AudioInputCreated { input })
    }

    pub fn remove_audio_input(&mut self, key: &str) -> Result<Change, StudioError> {
        let idx = self.audio_input_index(key)?;
        let removed = self.audio_inputs.remove(idx);
        Ok(Change::AudioInputRemoved {
            input_id: removed.id,
        })
    }

    /// Renomme une entrée, change son périphérique ou ses métadonnées.
    pub fn update_audio_input(
        &mut self,
        key: &str,
        patch: AudioInputPatch,
    ) -> Result<Change, StudioError> {
        if let Some(kind) = &patch.kind {
            Self::check_input_kind(kind)?;
        }
        if let Some(meta) = &patch.meta {
            check_meta(meta)?;
        }
        let idx = self.audio_input_index(key)?;
        if let Some(name) = &patch.name {
            if self
                .audio_inputs
                .iter()
                .enumerate()
                .any(|(i, x)| i != idx && &x.name == name)
            {
                return Err(StudioError::InputNameTaken(name.clone()));
            }
        }
        let input = &mut self.audio_inputs[idx];
        if let Some(name) = patch.name {
            input.name = name;
        }
        if let Some(kind) = patch.kind {
            input.kind = kind;
        }
        if let Some(meta) = patch.meta {
            input.meta = meta;
        }
        Ok(Change::AudioInputUpdated {
            input: input.clone(),
        })
    }

    pub fn set_audio_input_volume(
        &mut self,
        key: &str,
        volume: f32,
    ) -> Result<Change, StudioError> {
        check_volume(volume)?;
        let idx = self.audio_input_index(key)?;
        let input = &mut self.audio_inputs[idx];
        input.volume = volume;
        Ok(Change::AudioInputVolumeChanged {
            input_id: input.id,
            volume,
        })
    }

    pub fn set_audio_input_muted(&mut self, key: &str, muted: bool) -> Result<Change, StudioError> {
        let idx = self.audio_input_index(key)?;
        let input = &mut self.audio_inputs[idx];
        input.muted = muted;
        Ok(Change::AudioInputMuteChanged {
            input_id: input.id,
            muted,
        })
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

    #[test]
    fn audio_inputs_create_update_remove() {
        let mut s = Studio::default();
        let mic = SourceKind::AudioInput { device_id: None };
        s.create_audio_input("Micro Anne", mic.clone(), None, false, None)
            .unwrap();
        assert_eq!(
            s.create_audio_input("Micro Anne", mic.clone(), None, false, None),
            Err(StudioError::InputNameTaken("Micro Anne".into()))
        );
        assert_eq!(
            s.create_audio_input("Mire", SourceKind::TestPattern, None, false, None),
            Err(StudioError::InvalidInputKind)
        );
        assert_eq!(
            s.create_audio_input("Fort", mic.clone(), Some(3.0), false, None),
            Err(StudioError::InvalidVolume(3.0))
        );
        s.create_audio_input("Micro Paul", mic.clone(), Some(0.5), true, None)
            .unwrap();
        assert_eq!(s.find_audio_input("Micro Paul").unwrap().volume, 0.5);
        assert!(s.find_audio_input("Micro Paul").unwrap().muted);
        // Un renommage vers un nom pris est refusé.
        let taken = s.update_audio_input(
            "Micro Paul",
            AudioInputPatch {
                name: Some("Micro Anne".into()),
                ..Default::default()
            },
        );
        assert_eq!(taken, Err(StudioError::InputNameTaken("Micro Anne".into())));
        s.set_audio_input_muted("Micro Paul", false).unwrap();
        s.set_audio_input_volume("Micro Anne", 1.5).unwrap();
        assert!(!s.find_audio_input("Micro Paul").unwrap().muted);
        assert_eq!(s.find_audio_input("Micro Anne").unwrap().volume, 1.5);
        // Les entrées font partie des captures, en plus des sources des scènes.
        assert_eq!(s.capture_list().len(), 2);
        let id = s.find_audio_input("Micro Anne").unwrap().id;
        s.remove_audio_input(&id.to_string()).unwrap();
        assert_eq!(s.audio_inputs.len(), 1);
        assert!(matches!(
            s.remove_audio_input("Micro Anne"),
            Err(StudioError::InputNotFound(_))
        ));
    }

    #[test]
    fn meta_is_checked_and_patched() {
        let mut s = Studio::default();
        let bad = s.add_source_with_meta(
            "Scène 1",
            "Mire",
            SourceKind::TestPattern,
            None,
            Some(serde_json::json!("pas un objet")),
        );
        assert!(matches!(bad, Err(StudioError::InvalidMeta(_))));
        let big = "x".repeat(MAX_META_BYTES);
        let bad = s.add_source_with_meta(
            "Scène 1",
            "Mire",
            SourceKind::TestPattern,
            None,
            Some(serde_json::json!({ "v": big })),
        );
        assert!(matches!(bad, Err(StudioError::InvalidMeta(_))));
        s.add_source_with_meta(
            "Scène 1",
            "Mire",
            SourceKind::TestPattern,
            None,
            Some(serde_json::json!({ "owner": "nino" })),
        )
        .unwrap();
        assert_eq!(
            s.program().sources[0].meta.as_ref().unwrap()["owner"],
            "nino"
        );
        // `meta` absent : inchangé ; `meta: null` : effacé.
        let keep: SourcePatch =
            serde_json::from_value(serde_json::json!({ "visible": false })).unwrap();
        s.update_source("Scène 1", "Mire", keep).unwrap();
        assert!(s.program().sources[0].meta.is_some());
        let clear: SourcePatch =
            serde_json::from_value(serde_json::json!({ "meta": null })).unwrap();
        s.update_source("Scène 1", "Mire", clear).unwrap();
        assert!(s.program().sources[0].meta.is_none());
    }

    #[test]
    fn old_studio_json_without_audio_inputs_loads() {
        let s = Studio::default();
        let mut v = serde_json::to_value(&s).unwrap();
        v.as_object_mut().unwrap().remove("audio_inputs");
        let back: Studio = serde_json::from_value(v).unwrap();
        assert!(back.audio_inputs.is_empty());
    }
}
