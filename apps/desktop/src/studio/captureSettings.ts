/**
 * Réglages d'une source de capture qui, une fois changés, font relancer son
 * pipeline par le moteur (périphérique, URL, fichier…).
 */

import { isCaptureKind, type Source } from "@suite-media/client";

/** Champs propres à toute source, sans effet sur le pipeline de capture. */
const NON_CAPTURE_FIELDS = new Set(["id", "name", "transform", "visible", "volume", "muted"]);

/**
 * Empreinte des réglages de capture d'une source (type et champs du type, triés),
 * ou `null` si la source n'est pas une capture.
 */
export function captureSettingsKey(source: Source): string | null {
  if (!isCaptureKind(source.kind)) return null;
  const entries = Object.entries(source as Record<string, unknown>)
    .filter(([k, v]) => !NON_CAPTURE_FIELDS.has(k) && v !== undefined)
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return JSON.stringify(entries);
}

/** Effet d'un `SourceUpdated` sur l'état affiché de la capture. */
export type CaptureSettingsChange = "restarted" | "removed" | "unchanged";

/**
 * Compare les réglages connus avant un `SourceUpdated` (`undefined` : inconnus) à
 * ceux de la source mise à jour.
 */
export function captureSettingsChange(before: string | null | undefined, after: string | null): CaptureSettingsChange {
  if (before === undefined || before === after) return "unchanged";
  return after === null ? "removed" : "restarted";
}
