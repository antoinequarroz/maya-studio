/**
 * Calculs des VU-mètres, sans React : échelle en dBFS et balistique (montée
 * immédiate, descente progressive, maintien de la crête).
 */

/** Bas de l'échelle affichée, en dBFS. En dessous, le VU-mètre est vide. */
export const METER_FLOOR_DB = -60;

/** Graduations affichées sous les VU-mètres. */
export const METER_TICKS_DB = [-60, -50, -40, -30, -20, -10, 0] as const;

/** Seuils de couleur : vert jusqu'à -18 dBFS, jaune jusqu'à -6, rouge au-delà. */
export const METER_WARN_DB = -18;
export const METER_HOT_DB = -6;

/** Position (0 à 1) d'un niveau en dBFS sur l'échelle, linéaire en dB. */
export function dbToFraction(db: number, floor = METER_FLOOR_DB): number {
  if (!Number.isFinite(db) || db <= floor) return 0;
  if (db >= 0) return 1;
  return (db - floor) / -floor;
}

/** Vitesse de descente des barres, en dB par seconde. */
export const FALL_DB_PER_S = 24;
/** Durée de maintien de la crête avant qu'elle ne redescende, en ms. */
export const PEAK_HOLD_MS = 1200;
/** Vitesse de descente de la crête maintenue, en dB par seconde. */
export const PEAK_FALL_DB_PER_S = 18;

/** État affiché d'un canal : barres RMS et crête, et crête maintenue. */
export interface ChannelDisplay {
  rms: number;
  peak: number;
  hold: number;
  holdUntil: number;
}

export function silentChannel(): ChannelDisplay {
  return { rms: METER_FLOOR_DB, peak: METER_FLOOR_DB, hold: METER_FLOOR_DB, holdUntil: 0 };
}

/**
 * Fait avancer l'affichage d'un canal vers les derniers niveaux reçus.
 * Montée immédiate (on ne rate pas un transitoire), descente à {@link FALL_DB_PER_S}.
 */
export function stepChannel(
  current: ChannelDisplay,
  target: { rms: number; peak: number },
  dtMs: number,
  now: number,
): ChannelDisplay {
  const fall = (FALL_DB_PER_S * dtMs) / 1000;
  const follow = (shown: number, wanted: number) =>
    Math.max(METER_FLOOR_DB, wanted >= shown ? wanted : Math.max(wanted, shown - fall));
  const rms = follow(current.rms, target.rms);
  const peak = follow(current.peak, target.peak);
  let { hold, holdUntil } = current;
  if (peak >= hold) {
    hold = peak;
    holdUntil = now + PEAK_HOLD_MS;
  } else if (now > holdUntil) {
    hold = Math.max(peak, hold - (PEAK_FALL_DB_PER_S * dtMs) / 1000);
  }
  return { rms, peak, hold, holdUntil };
}

/** Niveau le plus fort d'une liste de canaux (plancher si vide). */
export function loudest(values: readonly number[] | undefined): number {
  if (!values || values.length === 0) return METER_FLOOR_DB;
  return Math.max(...values);
}

/** Volume linéaire (0 à 2) vers pourcentage entier pour le curseur. */
export function volumeToPercent(volume: number | undefined): number {
  return Math.round((volume ?? 1) * 100);
}

/** Pourcentage du curseur vers volume linéaire, borné à 0..2. */
export function percentToVolume(percent: number): number {
  return Math.min(2, Math.max(0, Math.round(percent) / 100));
}

/** Gain en dB lisible (« -6,0 dB », « −∞ dB » pour 0). */
export function formatGainDb(volume: number): string {
  if (volume <= 0) return "−∞ dB";
  const db = 20 * Math.log10(volume);
  const text = Math.abs(db) < 0.05 ? "0,0" : db.toFixed(1).replace(".", ",").replace("-", "−");
  return `${db > 0.05 ? "+" : ""}${text} dB`;
}
