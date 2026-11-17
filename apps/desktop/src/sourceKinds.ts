/** Libellés et résumés des types de source, partagés par les panneaux. */

import type { Source, SourceKindName } from "@suite-media/client";

export const KIND_LABELS: Record<SourceKindName, string> = {
  color: "Couleur",
  image: "Image",
  test_pattern: "Mire",
  camera: "Caméra",
  display_capture: "Écran",
  window_capture: "Fenêtre",
  audio_input: "Entrée audio",
  media_file: "Fichier vidéo",
  srt_input: "Flux SRT",
};

/** Dernier segment d'un chemin ou d'une URI, pour un affichage court. */
export function baseName(path: string): string {
  const clean = path.replace(/[?#].*$/, "").replace(/[\\/]+$/, "");
  return clean.split(/[\\/]/).pop() || path;
}

/** Résumé en une ligne des réglages d'une source (infobulle de la liste). */
export function sourceSummary(source: Source, deviceName?: (id: string) => string | undefined): string {
  const device = (id: string | undefined, fallback: string) => (id ? (deviceName?.(id) ?? id) : fallback);
  switch (source.kind) {
    case "image":
      return source.path;
    case "media_file":
      return `${source.path}${source.loop ? " (en boucle)" : ""}`;
    case "srt_input":
      return source.url;
    case "camera":
      return device(source.device_id, "Caméra par défaut");
    case "audio_input":
      return device(source.device_id, "Entrée audio par défaut");
    case "display_capture":
      return source.display_id ? `Écran ${source.display_id}` : "Écran principal";
    case "window_capture":
      return source.window_id ? `Fenêtre ${source.window_id}` : "Fenêtre";
    default:
      return KIND_LABELS[source.kind];
  }
}
