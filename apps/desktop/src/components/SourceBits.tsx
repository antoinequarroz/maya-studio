/** Petits éléments communs aux panneaux de sources : icône du type, état de capture. */

import { type InputDevices, isCaptureKind, type Source, type SourceKindName } from "@suite-media/client";
import { useCallback, useEffect, useState } from "react";
import { KIND_LABELS } from "../sourceKinds";
import { type CaptureStatus, useStudio } from "../studio/StudioContext";
import { rgbaToCss } from "./color";
import {
  IconCamera,
  IconFilm,
  IconImage,
  IconMic,
  IconPattern,
  IconScreen,
  IconSignal,
  IconWindow,
} from "./Icons";

export const KIND_ICONS: Record<Exclude<SourceKindName, "color">, typeof IconImage> = {
  image: IconImage,
  test_pattern: IconPattern,
  camera: IconCamera,
  display_capture: IconScreen,
  window_capture: IconWindow,
  audio_input: IconMic,
  media_file: IconFilm,
  srt_input: IconSignal,
};

export function KindIcon({ source }: { source: Source }) {
  if (source.kind === "color") return <span className="swatch" style={{ background: rgbaToCss(source.rgba) }} />;
  const Icon = KIND_ICONS[source.kind];
  return <Icon />;
}

/** Texte de l'infobulle d'une capture en échec. */
export function failureTooltip(status: Extract<CaptureStatus, { state: "failed" }>): string {
  const retry = Math.max(1, Math.round(status.retryInMs / 1000));
  return `${status.error}\nNouvel essai automatique ${retry} s après l'échec (délai croissant, 30 s au plus).`;
}

/**
 * Pastille d'état d'une source de capture : « démarrage », « en marche » ou « en échec »
 * (détail en infobulle). Rien pour les sources sans capture ni pour un état inconnu.
 */
export function SourceStatusBadge({ source, compact = false }: { source: Source; compact?: boolean }) {
  const { captures } = useStudio();
  if (!isCaptureKind(source.kind)) return null;
  const status = captures[source.id];
  if (!status) return null;
  if (status.state === "starting") {
    return (
      <span className={`status-badge pending${compact ? " compact" : ""}`} title={`${KIND_LABELS[source.kind]} : en attente des premières images ou du premier son`}>
        <span className="dot" />
        {!compact && "démarrage"}
      </span>
    );
  }
  if (status.state === "running") {
    return (
      <span className={`status-badge ok${compact ? " compact" : ""}`} title={`${KIND_LABELS[source.kind]} : capture en marche`}>
        <span className="dot" />
        {!compact && "en marche"}
      </span>
    );
  }
  return (
    <span className={`status-badge failed${compact ? " compact" : ""}`} title={failureTooltip(status)}>
      <span className="dot" />
      {!compact && "en échec"}
    </span>
  );
}

/** Périphériques d'entrée du moteur (`GetInputDevices`), rechargeables à la demande. */
export function useInputDevices(): { devices: InputDevices | null; loading: boolean; reload: () => void } {
  const { client, connection } = useStudio();
  const [devices, setDevices] = useState<InputDevices | null>(null);
  const [loading, setLoading] = useState(false);
  const reload = useCallback(() => {
    setLoading(true);
    client
      .call("GetInputDevices")
      .then(setDevices, () => setDevices({ video: [], audio: [] }))
      .finally(() => setLoading(false));
  }, [client]);
  useEffect(() => {
    if (connection === "connected") reload();
  }, [connection, reload]);
  return { devices, loading, reload };
}
