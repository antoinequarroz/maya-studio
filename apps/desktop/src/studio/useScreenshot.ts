/**
 * Image d'un moniteur, par interrogation de `GetScreenshot` en JPEG (~5 i/s).
 *
 * C'est volontairement simple et provisoire : un vrai flux d'aperçu à faible
 * latence (WebRTC / MJPEG / mémoire partagée côté Tauri) fera l'objet d'un ticket dédié.
 */

import { useEffect, useState } from "react";
import { useStudio } from "./StudioContext";

export interface MonitorFrame {
  /** Data URL JPEG de la dernière image, ou null. */
  src: string | null;
  /** Cadence mesurée de rafraîchissement, en images par seconde. */
  fps: number;
}

export function useScreenshot(sceneId: string | null, intervalMs = 200): MonitorFrame {
  const { client, connection } = useStudio();
  const [frame, setFrame] = useState<MonitorFrame>({ src: null, fps: 0 });

  useEffect(() => {
    if (!sceneId || connection !== "connected") return;
    let stopped = false;
    let timer: number | undefined;
    let count = 0;
    let windowStart = performance.now();
    let fps = 0;

    const tick = async () => {
      const t0 = performance.now();
      try {
        const { image } = await client.call("GetScreenshot", { scene: sceneId, format: "jpeg" }, { timeoutMs: 3000 });
        if (stopped) return;
        count += 1;
        const now = performance.now();
        if (now - windowStart >= 1000) {
          fps = (count * 1000) / (now - windowStart);
          count = 0;
          windowStart = now;
        }
        setFrame({ src: image, fps });
      } catch {
        // scène supprimée ou coupure : on réessaie au prochain tour
      }
      if (stopped) return;
      const wait = Math.max(0, intervalMs - (performance.now() - t0));
      timer = window.setTimeout(tick, wait);
    };
    void tick();
    return () => {
      stopped = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [client, connection, sceneId, intervalMs]);

  return frame;
}
