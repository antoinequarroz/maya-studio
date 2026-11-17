//! Mixage audio : calculs purs (somme, gain, mute, écrêtage, niveaux en dBFS).
//!
//! Le format est celui des sorties : S16 entrelacé, 48 kHz, stéréo. Le thread
//! du mixeur (voir `engine`) tire un bloc de 10 ms de chaque source, les somme
//! ici et pousse le résultat dans l'entrée audio de chaque sortie active.

use crate::outputs::{AUDIO_CHANNELS, AUDIO_RATE};

/// Durée d'un bloc de mixage.
pub const BLOCK_MS: u32 = 10;
/// Échantillons par canal dans un bloc (480 à 48 kHz).
pub const BLOCK_FRAMES: usize = (AUDIO_RATE * BLOCK_MS / 1000) as usize;
/// Échantillons entrelacés dans un bloc.
pub const BLOCK_SAMPLES: usize = BLOCK_FRAMES * AUDIO_CHANNELS as usize;
/// Plancher des niveaux : le silence numérique vaut -100 dBFS (JSON n'a pas -∞).
pub const MIN_DB: f32 = -100.0;

/// Accumulateur d'un bloc de mixage, en flottant pour ne pas écrêter avant la fin.
pub struct MixBus {
    acc: Vec<f32>,
}

impl MixBus {
    pub fn new(samples: usize) -> Self {
        Self {
            acc: vec![0.0; samples],
        }
    }

    /// Remet le bloc à zéro (silence).
    pub fn clear(&mut self) {
        self.acc.iter_mut().for_each(|s| *s = 0.0);
    }

    /// Ajoute une source avec son gain linéaire (borné à 0..2). Une source coupée
    /// n'est simplement pas ajoutée (voir [`MixBus::add_source`]).
    pub fn add(&mut self, input: &[i16], volume: f32) {
        let gain = clamp_volume(volume);
        if gain == 0.0 {
            return;
        }
        for (a, &s) in self.acc.iter_mut().zip(input) {
            *a += f32::from(s) * gain;
        }
    }

    /// Ajoute une source en tenant compte de son mute.
    pub fn add_source(&mut self, input: &[i16], volume: f32, muted: bool) {
        if !muted {
            self.add(input, volume);
        }
    }

    /// Résultat du mixage, écrêté dans la plage S16.
    pub fn output(&self, out: &mut [i16]) {
        for (o, &a) in out.iter_mut().zip(&self.acc) {
            *o = clamp_sample(a);
        }
    }
}

/// Gain borné à la plage autorisée ; NaN donne 0.
pub fn clamp_volume(volume: f32) -> f32 {
    if volume.is_nan() {
        0.0
    } else {
        volume.clamp(0.0, studio_core::MAX_VOLUME)
    }
}

/// Arrondi et écrêtage d'un échantillon mixé.
pub fn clamp_sample(v: f32) -> i16 {
    v.round().clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

/// Amplitude linéaire (1.0 = pleine échelle) en dBFS, avec plancher à [`MIN_DB`].
pub fn to_dbfs(amplitude: f32) -> f32 {
    if amplitude <= 0.0 || amplitude.is_nan() {
        return MIN_DB;
    }
    (20.0 * amplitude.log10()).max(MIN_DB)
}

/// Mesure de niveau par canal, accumulée sur plusieurs blocs (crête et RMS).
#[derive(Debug, Clone, Default)]
pub struct LevelMeter {
    peak: [f32; AUDIO_CHANNELS as usize],
    sum_sq: [f64; AUDIO_CHANNELS as usize],
    frames: u64,
}

/// Niveaux d'une source sur une fenêtre, en dBFS, par canal (gauche, droite).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Levels {
    pub peak_db: [f32; AUDIO_CHANNELS as usize],
    pub rms_db: [f32; AUDIO_CHANNELS as usize],
}

impl LevelMeter {
    /// Ajoute un bloc entrelacé, mesuré après application du gain.
    pub fn add(&mut self, interleaved: &[i16], volume: f32) {
        let gain = clamp_volume(volume);
        let ch = AUDIO_CHANNELS as usize;
        for frame in interleaved.chunks_exact(ch) {
            for (c, &s) in frame.iter().enumerate() {
                // Mesure après gain mais avant écrêtage : au-dessus de 0 dBFS, ça sature.
                let v = (f32::from(s) * gain / 32768.0).abs();
                if v > self.peak[c] {
                    self.peak[c] = v;
                }
                self.sum_sq[c] += f64::from(v) * f64::from(v);
            }
            self.frames += 1;
        }
    }

    /// Niveaux depuis la dernière remise à zéro, puis remise à zéro.
    pub fn take(&mut self) -> Levels {
        let mut levels = Levels {
            peak_db: [MIN_DB; AUDIO_CHANNELS as usize],
            rms_db: [MIN_DB; AUDIO_CHANNELS as usize],
        };
        for c in 0..AUDIO_CHANNELS as usize {
            levels.peak_db[c] = to_dbfs(self.peak[c]);
            if self.frames > 0 {
                levels.rms_db[c] = to_dbfs((self.sum_sq[c] / self.frames as f64).sqrt() as f32);
            }
        }
        *self = Self::default();
        levels
    }
}

/// Avance maximale de la piste audio sur la vidéo d'une sortie (100 ms).
pub const MAX_AUDIO_LEAD: u64 = AUDIO_RATE as u64 / 10;
/// Retard maximal de la piste audio sur la vidéo avant de combler par du silence (100 ms).
pub const MAX_AUDIO_LAG: u64 = AUDIO_RATE as u64 / 10;

/// Calage A/V d'une sortie : les deux pistes sont horodatées par compteurs à
/// partir de 0 (images pour la vidéo, échantillons pour l'audio). Pour que l'audio
/// suive la même horloge que les PTS vidéo, chaque bloc est placé par rapport à
/// la position vidéo `video_frames / fps`, et non par rapport à l'horloge murale.
///
/// Renvoie `None` s'il faut sauter ce bloc (vidéo pas encore démarrée, ou audio
/// trop en avance parce que le rendu a pris du retard), sinon le nombre
/// d'échantillons de silence à insérer avant le bloc (audio trop en retard).
pub fn plan_block(video_frames: u64, fps: u32, audio_pos: u64) -> Option<u64> {
    if video_frames == 0 {
        return None;
    }
    let video_pos = video_frames * u64::from(AUDIO_RATE) / u64::from(fps.max(1));
    if audio_pos >= video_pos + MAX_AUDIO_LEAD {
        return None;
    }
    if audio_pos + MAX_AUDIO_LAG < video_pos {
        return Some(video_pos - audio_pos - BLOCK_FRAMES as u64);
    }
    Some(0)
}

/// Arrondi des niveaux au dixième de dB, assez pour un VU-mètre et plus léger en JSON.
/// Renvoie un f64 pour que le JSON affiche `-7.9` et non `-7.900000095367432`.
pub fn round_db(db: f32) -> f64 {
    (f64::from(db) * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mix(inputs: &[(&[i16], f32, bool)]) -> Vec<i16> {
        let n = inputs.iter().map(|i| i.0.len()).max().unwrap_or(0);
        let mut bus = MixBus::new(n);
        for (input, volume, muted) in inputs {
            bus.add_source(input, *volume, *muted);
        }
        let mut out = vec![0; n];
        bus.output(&mut out);
        out
    }

    #[test]
    fn block_size() {
        assert_eq!(BLOCK_FRAMES, 480);
        assert_eq!(BLOCK_SAMPLES, 960);
    }

    #[test]
    fn sums_sources() {
        let a = [1000, -1000, 200, 0];
        let b = [500, 500, -200, 7];
        assert_eq!(
            mix(&[(&a, 1.0, false), (&b, 1.0, false)]),
            [1500, -500, 0, 7]
        );
        // Aucune source : silence.
        let mut bus = MixBus::new(4);
        let mut out = [9i16; 4];
        bus.output(&mut out);
        assert_eq!(out, [0; 4]);
        bus.add(&a, 1.0);
        bus.clear();
        bus.output(&mut out);
        assert_eq!(out, [0; 4]);
    }

    #[test]
    fn applies_volume() {
        let a = [1000, -1000, 3];
        assert_eq!(mix(&[(&a, 0.5, false)]), [500, -500, 2]);
        assert_eq!(mix(&[(&a, 2.0, false)]), [2000, -2000, 6]);
        assert_eq!(mix(&[(&a, 0.0, false)]), [0, 0, 0]);
        // Gain hors plage : borné à 2, NaN coupe.
        assert_eq!(mix(&[(&a, 10.0, false)]), [2000, -2000, 6]);
        assert_eq!(mix(&[(&a, f32::NAN, false)]), [0, 0, 0]);
        assert_eq!(mix(&[(&a, -1.0, false)]), [0, 0, 0]);
    }

    #[test]
    fn mute_removes_source() {
        let a = [1000, 1000];
        let b = [10, 20];
        assert_eq!(mix(&[(&a, 1.0, true), (&b, 1.0, false)]), [10, 20]);
        assert_eq!(mix(&[(&a, 1.0, true)]), [0, 0]);
    }

    #[test]
    fn clamps_to_s16() {
        let a = [30000, -30000, i16::MAX, i16::MIN];
        assert_eq!(
            mix(&[(&a, 1.0, false), (&a, 1.0, false)]),
            [i16::MAX, i16::MIN, i16::MAX, i16::MIN]
        );
        assert_eq!(mix(&[(&a, 2.0, false)])[0], i16::MAX);
        assert_eq!(clamp_sample(1e9), i16::MAX);
        assert_eq!(clamp_sample(-1e9), i16::MIN);
        assert_eq!(clamp_sample(0.4), 0);
    }

    #[test]
    fn av_sync_plan() {
        // Vidéo pas commencée : pas d'audio.
        assert_eq!(plan_block(0, 30, 0), None);
        // Régime normal : on pousse sans combler.
        assert_eq!(plan_block(1, 30, 0), Some(0));
        assert_eq!(plan_block(30, 30, 47_000), Some(0));
        // Audio trop en avance (rendu en retard) : bloc sauté.
        assert_eq!(plan_block(30, 30, 48_000 + MAX_AUDIO_LEAD), None);
        // Audio trop en retard : silence pour se recaler sur la vidéo.
        let gap = plan_block(60, 30, 10_000).unwrap();
        assert_eq!(10_000 + gap + BLOCK_FRAMES as u64, 96_000);
        // Simulation : 2 s de vidéo à 30 i/s et de blocs de 10 ms au même rythme
        // donnent la même durée audio et vidéo.
        let (mut frames, mut audio) = (0u64, 0u64);
        for ms in 0..2000u64 {
            if ms % 10 == 0 {
                if let Some(s) = plan_block(frames, 30, audio) {
                    audio += s + BLOCK_FRAMES as u64;
                }
            }
            if ms * 30 / 1000 >= frames {
                frames += 1;
            }
        }
        let video_pos = frames * 48_000 / 30;
        assert!(
            audio.abs_diff(video_pos) <= 1600 + BLOCK_FRAMES as u64,
            "{audio} / {video_pos}"
        );
    }

    #[test]
    fn dbfs() {
        assert_eq!(to_dbfs(1.0), 0.0);
        assert!((to_dbfs(0.5) - -6.0206).abs() < 1e-3);
        assert!((to_dbfs(0.1) - -20.0).abs() < 1e-4);
        assert_eq!(to_dbfs(0.0), MIN_DB);
        assert_eq!(to_dbfs(1e-9), MIN_DB);
        assert_eq!(to_dbfs(f32::NAN), MIN_DB);
        assert_eq!(round_db(-6.0206), -6.0);
    }

    #[test]
    fn level_meter_peak_and_rms() {
        let mut m = LevelMeter::default();
        // Silence : plancher.
        m.add(&[0; 960], 1.0);
        let l = m.take();
        assert_eq!(l.peak_db, [MIN_DB, MIN_DB]);
        assert_eq!(l.rms_db, [MIN_DB, MIN_DB]);
        // Carré pleine échelle à gauche (crête = RMS = 0 dBFS), demi-échelle à droite.
        let block: Vec<i16> = (0..480)
            .flat_map(|i| {
                let s = if i % 2 == 0 { i16::MIN } else { i16::MIN + 1 };
                [s, -16384]
            })
            .collect();
        m.add(&block, 1.0);
        let l = m.take();
        assert!(l.peak_db[0].abs() < 0.01, "{:?}", l);
        assert!(l.rms_db[0].abs() < 0.01, "{:?}", l);
        assert!((l.peak_db[1] - -6.02).abs() < 0.01, "{:?}", l);
        assert!((l.rms_db[1] - -6.02).abs() < 0.01, "{:?}", l);
        // Le gain s'applique à la mesure ; take() remet à zéro.
        m.add(&block, 0.5);
        let l = m.take();
        assert!((l.peak_db[0] - -6.02).abs() < 0.01, "{:?}", l);
        assert_eq!(m.take().peak_db, [MIN_DB, MIN_DB]);
        // Sinusoïde : RMS = crête - 3 dB.
        let sine: Vec<i16> = (0..4800)
            .flat_map(|i| {
                let v = (16384.0 * (i as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 48000.0).sin())
                    as i16;
                [v, v]
            })
            .collect();
        m.add(&sine, 1.0);
        let l = m.take();
        assert!((l.peak_db[0] - l.rms_db[0] - 3.01).abs() < 0.05, "{:?}", l);
    }
}
