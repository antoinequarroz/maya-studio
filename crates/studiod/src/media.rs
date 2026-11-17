//! Dernières images et son des sources de capture, partagés entre les pipelines
//! de capture (qui écrivent), la boucle de rendu et le mixeur audio (qui lisent).
//!
//! Ce module ne dépend pas de GStreamer : sans la fonctionnalité `gstreamer`,
//! le magasin reste simplement vide.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Rattrapage de la dérive d'horloge : nombre de lectures (10 ms chacune pour
/// le mixeur) sur lesquelles on observe le remplissage minimal de la file.
pub const DRIFT_WINDOW: u32 = 100;
/// En dessous de ce remplissage minimal (après lecture), la source prend du
/// retard sur le mixeur : on duplique une trame par lecture pendant la fenêtre
/// suivante pour regagner de la marge avant le manque.
pub const DRIFT_LOW: usize = samples_for_ms(5);
/// Au-dessus, la source prend de l'avance : la latence grimpe, on jette une
/// trame par lecture pendant la fenêtre suivante.
pub const DRIFT_HIGH: usize = samples_for_ms(30);

/// Correction appliquée à chaque lecture pendant la fenêtre en cours.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum DriftCorrection {
    #[default]
    None,
    /// Une trame de moins lue dans la file, la dernière est répétée.
    Stretch,
    /// Une trame de plus retirée de la file.
    Skip,
}

#[derive(Default)]
struct AudioQueue {
    samples: VecDeque<i16>,
    /// Réserve constituée : on lit. Repasse à faux après un manque.
    primed: bool,
    /// Lectures depuis le début de la fenêtre d'observation.
    window_reads: u32,
    /// Remplissage minimal (après lecture) observé dans la fenêtre.
    window_min: usize,
    correction: DriftCorrection,
}

impl AudioQueue {
    fn reset_drift(&mut self) {
        self.window_reads = 0;
        self.window_min = usize::MAX;
        self.correction = DriftCorrection::None;
    }

    /// Fin de lecture : met à jour l'observation et choisit la correction de la
    /// fenêtre suivante. Corriger d'une trame par lecture de 10 ms (environ
    /// 2000 ppm au plus) suffit largement pour les écarts d'horloge courants
    /// (10 à 100 ppm) et reste inaudible, contrairement au silence d'un manque
    /// ou au saut d'un débordement.
    fn observe_fill(&mut self) {
        self.window_min = self.window_min.min(self.samples.len());
        self.window_reads += 1;
        if self.window_reads >= DRIFT_WINDOW {
            self.correction = if self.window_min < DRIFT_LOW {
                DriftCorrection::Stretch
            } else if self.window_min > DRIFT_HIGH {
                DriftCorrection::Skip
            } else {
                DriftCorrection::None
            };
            self.window_reads = 0;
            self.window_min = usize::MAX;
        }
    }
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
            q.reset_drift();
        }
        let ch = AUDIO_CHANNELS as usize;
        let stretch = q.correction == DriftCorrection::Stretch && out.len() >= 2 * ch;
        // Étirement : une trame de moins prise dans la file, la dernière répétée.
        let wanted = if stretch { out.len() - ch } else { out.len() };
        let n = wanted.min(q.samples.len());
        for (o, s) in out.iter_mut().zip(q.samples.drain(..n)) {
            *o = s;
        }
        if n < wanted {
            out[n..].iter_mut().for_each(|s| *s = 0);
            // Manque : on reconstitue une réserve avant de reprendre.
            q.primed = false;
            return n > 0;
        }
        if stretch {
            let (head, tail) = out.split_at_mut(wanted);
            tail.copy_from_slice(&head[wanted - ch..]);
        }
        if q.correction == DriftCorrection::Skip && q.samples.len() >= ch {
            // Une trame sautée : la file se vide un peu plus vite que la source.
            q.samples.drain(..ch);
        }
        q.observe_fill();
        n > 0
    }

    /// Vide la file (redémarrage ou bouclage de la source).
    pub fn clear(&self) {
        let mut q = self.inner.lock().unwrap();
        q.samples.clear();
        q.primed = false;
        q.reset_drift();
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
    /// Une image ou du son est arrivé depuis le dernier démarrage du pipeline.
    data_seen: AtomicBool,
}

impl SourceMedia {
    pub fn new(with_audio: bool) -> Self {
        Self {
            video: Mutex::new(None),
            audio: with_audio.then(AudioBuffer::default),
            data_seen: AtomicBool::new(false),
        }
    }

    pub fn set_frame(&self, frame: VideoFrame) {
        *self.video.lock().unwrap() = Some(Arc::new(frame));
        self.mark_data_seen();
    }

    /// Signale qu'une image ou du son a été reçu.
    pub fn mark_data_seen(&self) {
        self.data_seen.store(true, Ordering::Relaxed);
    }

    pub fn data_seen(&self) -> bool {
        self.data_seen.load(Ordering::Relaxed)
    }

    /// Remet l'indicateur à zéro avant un (re)démarrage du pipeline.
    pub fn reset_data_seen(&self) {
        self.data_seen.store(false, Ordering::Relaxed);
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

    /// Simule une source dont l'horloge s'écarte de `ppm` de celle du mixeur,
    /// livrant par paquets de 20 ms, pendant `minutes`. Renvoie le nombre de
    /// lectures incomplètes et le remplissage maximal observé.
    fn simulate_drift(ppm: f64, minutes: u32) -> (usize, usize) {
        let b = AudioBuffer::default();
        let block = samples_for_ms(10);
        let packet = samples_for_ms(20) as f64 * (1.0 + ppm / 1e6);
        let mut out = vec![0i16; block];
        let mut owed = 0.0f64;
        let mut short_reads = 0;
        let mut max_fill = 0;
        let mut started = false;
        for tick in 0..(minutes * 60 * 100) {
            // Toutes les 20 ms, un paquet ; l'écart d'horloge s'accumule en fractions.
            if tick % 2 == 0 {
                owed += packet;
                let frames = (owed / AUDIO_CHANNELS as f64).floor() as usize;
                let n = frames * AUDIO_CHANNELS as usize;
                owed -= n as f64;
                b.push(&vec![1; n]);
            }
            max_fill = max_fill.max(b.len());
            let full = b.pull(&mut out) && out.iter().all(|&s| s == 1);
            if full {
                started = true;
            } else if started {
                short_reads += 1;
            }
        }
        (short_reads, max_fill)
    }

    #[test]
    fn slow_device_never_underruns() {
        // 100 ppm de retard : sans correction, la réserve de 40 ms est épuisée
        // en quelques minutes et un trou de silence apparaît.
        let (short, _) = simulate_drift(-100.0, 15);
        assert_eq!(short, 0, "aucun trou de silence attendu");
    }

    #[test]
    fn fast_device_latency_stays_bounded() {
        // 100 ppm d'avance : sans correction, la file grimpe jusqu'à 200 ms.
        let (short, max_fill) = simulate_drift(100.0, 15);
        assert_eq!(short, 0);
        assert!(
            max_fill < samples_for_ms(80),
            "latence bornée : {max_fill} échantillons en attente"
        );
    }

    #[test]
    fn stretch_repeats_last_frame() {
        let b = AudioBuffer::default();
        let mut out = vec![0i16; 960];
        b.push(&(0..AUDIO_PRIME as i16 * 4).collect::<Vec<_>>());
        b.inner.lock().unwrap().primed = true;
        b.inner.lock().unwrap().correction = DriftCorrection::Stretch;
        assert!(b.pull(&mut out));
        assert_eq!(out[956..], [956, 957, 956, 957]);
        assert_eq!(b.len(), AUDIO_PRIME * 4 - 958);
        b.inner.lock().unwrap().correction = DriftCorrection::Skip;
        assert!(b.pull(&mut out));
        assert_eq!(out[0], 958);
        assert_eq!(b.len(), AUDIO_PRIME * 4 - 958 - 962);
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
