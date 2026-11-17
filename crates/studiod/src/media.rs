//! Dernières images et son des sources de capture, partagés entre les pipelines
//! de capture (qui écrivent), la boucle de rendu et le mixeur audio (qui lisent).
//!
//! Ce module ne dépend pas de GStreamer : sans la fonctionnalité `gstreamer`,
//! le magasin reste simplement vide.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};

use studio_core::{FrameProvider, VideoFrame};
use uuid::Uuid;

use crate::outputs::{AUDIO_CHANNELS, AUDIO_RATE};

/// Échantillons entrelacés pour `ms` millisecondes de son.
const fn samples_for_ms(ms: u32) -> usize {
    (AUDIO_RATE / 1000 * ms * AUDIO_CHANNELS) as usize
}

/// Son en attente au plus par source : au-delà, le plus ancien est jeté
/// (la latence reste bornée si la source va plus vite que l'horloge du mixeur).
pub const AUDIO_MAX_BUFFER: usize = samples_for_ms(200);
/// Réserve à constituer avant de lire une source (absorbe la gigue des captures
/// et des décodeurs qui livrent par paquets de 20 à 40 ms).
pub const AUDIO_PRIME: usize = samples_for_ms(40);

#[derive(Default)]
struct AudioQueue {
    samples: VecDeque<i16>,
    /// Réserve constituée : on lit. Repasse à faux après un manque.
    primed: bool,
}

/// File de son d'une source : S16 entrelacé, 48 kHz, stéréo.
#[derive(Default)]
pub struct AudioBuffer {
    inner: Mutex<AudioQueue>,
}

impl AudioBuffer {
    /// Ajoute des échantillons reçus de la capture.
    pub fn push(&self, interleaved: &[i16]) {
        let mut q = self.inner.lock().unwrap();
        q.samples.extend(interleaved);
        if q.samples.len() > AUDIO_MAX_BUFFER {
            // On garde l'alignement des canaux en jetant un nombre pair d'échantillons.
            let mut excess = q.samples.len() - AUDIO_MAX_BUFFER;
            excess += (q.samples.len() - excess) % AUDIO_CHANNELS as usize;
            let excess = excess.min(q.samples.len());
            q.samples.drain(..excess);
        }
    }

    /// Remplit `out` avec le son suivant, complété par du silence s'il en manque.
    /// Renvoie faux si la source n'a rien fourni (réserve en cours de constitution).
    pub fn pull(&self, out: &mut [i16]) -> bool {
        let mut q = self.inner.lock().unwrap();
        if !q.primed {
            if q.samples.len() < AUDIO_PRIME.max(out.len()) {
                out.iter_mut().for_each(|s| *s = 0);
                return false;
            }
            q.primed = true;
        }
        let n = out.len().min(q.samples.len());
        for (o, s) in out.iter_mut().zip(q.samples.drain(..n)) {
            *o = s;
        }
        out[n..].iter_mut().for_each(|s| *s = 0);
        if n < out.len() {
            // Manque : on reconstitue une réserve avant de reprendre.
            q.primed = false;
        }
        n > 0
    }

    /// Vide la file (redémarrage ou bouclage de la source).
    pub fn clear(&self) {
        let mut q = self.inner.lock().unwrap();
        q.samples.clear();
        q.primed = false;
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Médias d'une source de capture.
#[derive(Default)]
pub struct SourceMedia {
    /// Dernière image reçue (RGBA), remplacée à chaque nouvelle image.
    pub video: Mutex<Option<Arc<VideoFrame>>>,
    /// Son en attente du mixeur, pour les sources qui en produisent.
    pub audio: Option<AudioBuffer>,
}

impl SourceMedia {
    pub fn new(with_audio: bool) -> Self {
        Self {
            video: Mutex::new(None),
            audio: with_audio.then(AudioBuffer::default),
        }
    }

    pub fn set_frame(&self, frame: VideoFrame) {
        *self.video.lock().unwrap() = Some(Arc::new(frame));
    }
}

/// Médias de toutes les sources de capture, par identifiant de source.
#[derive(Default)]
pub struct MediaStore {
    sources: RwLock<HashMap<Uuid, Arc<SourceMedia>>>,
}

impl MediaStore {
    pub fn insert(&self, id: Uuid, media: Arc<SourceMedia>) {
        self.sources.write().unwrap().insert(id, media);
    }

    pub fn remove(&self, id: Uuid) {
        self.sources.write().unwrap().remove(&id);
    }

    pub fn get(&self, id: Uuid) -> Option<Arc<SourceMedia>> {
        self.sources.read().unwrap().get(&id).cloned()
    }

    /// Sources qui ont une file audio.
    pub fn audio_sources(&self) -> Vec<(Uuid, Arc<SourceMedia>)> {
        self.sources
            .read()
            .unwrap()
            .iter()
            .filter(|(_, m)| m.audio.is_some())
            .map(|(id, m)| (*id, Arc::clone(m)))
            .collect()
    }
}

impl FrameProvider for MediaStore {
    fn frame(&self, source_id: Uuid) -> Option<Arc<VideoFrame>> {
        self.get(source_id)?.video.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_buffer_primes_then_reads() {
        let b = AudioBuffer::default();
        let mut out = [7i16; 960];
        // Pas assez pour la réserve : silence.
        b.push(&[1; 960]);
        assert!(!b.pull(&mut out));
        assert!(out.iter().all(|&s| s == 0));
        b.push(&vec![2; AUDIO_PRIME]);
        assert!(b.pull(&mut out));
        assert!(out.iter().all(|&s| s == 1));
        // Lecture jusqu'au manque, complété par du silence.
        let mut big = vec![0i16; AUDIO_PRIME + 2];
        assert!(b.pull(&mut big));
        assert_eq!(big[0], 2);
        assert_eq!(*big.last().unwrap(), 0, "manque complété par du silence");
        // Après un manque, il faut reconstituer la réserve.
        b.push(&[3; 960]);
        assert!(!b.pull(&mut out));
        b.clear();
        assert!(b.is_empty());
    }

    #[test]
    fn audio_buffer_bounded() {
        let b = AudioBuffer::default();
        b.push(&vec![0; AUDIO_MAX_BUFFER + 3]);
        assert!(b.len() <= AUDIO_MAX_BUFFER);
        assert_eq!(b.len() % AUDIO_CHANNELS as usize, 0);
    }

    #[test]
    fn store_provides_frames() {
        let store = MediaStore::default();
        let id = Uuid::new_v4();
        assert!(store.frame(id).is_none());
        let media = Arc::new(SourceMedia::new(true));
        store.insert(id, Arc::clone(&media));
        assert!(store.frame(id).is_none());
        media.set_frame(VideoFrame::new(1, 1, vec![1, 2, 3, 4]).unwrap());
        assert_eq!(store.frame(id).unwrap().data, [1, 2, 3, 4]);
        assert_eq!(store.audio_sources().len(), 1);
        store.remove(id);
        assert!(store.frame(id).is_none());
    }
}
