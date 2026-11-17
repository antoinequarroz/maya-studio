/** Mode studio : moniteurs Aperçu et Programme côte à côte, et bouton Couper. */

import { findScene } from "@suite-media/client";
import type { CSSProperties } from "react";
import { useStudio } from "../studio/StudioContext";
import { useScreenshot } from "../studio/useScreenshot";

function Monitor({ role, sceneId }: { role: "preview" | "program"; sceneId: string | null }) {
  const { state, outputs } = useStudio();
  const frame = useScreenshot(sceneId);
  const scene = state && sceneId ? findScene(state, sceneId) : undefined;
  const ratio = state ? state.canvas.width / state.canvas.height : 16 / 9;
  const live = role === "program" && !!(outputs?.stream.active || outputs?.record.active);

  return (
    <section className={`monitor monitor-${role}`} aria-label={role === "program" ? "Programme" : "Aperçu"}>
      <header className="monitor-header">
        <span className={`tally tally-${role}`}>{role === "program" ? "Programme" : "Aperçu"}</span>
        <span className="monitor-scene" title={scene?.name}>
          {scene?.name ?? "—"}
        </span>
        {live && <span className="badge-onair">À l'antenne</span>}
        <span className="monitor-fps" title="Cadence de rafraîchissement du moniteur">
          {frame.fps > 0 ? `${frame.fps.toFixed(1)} i/s` : ""}
        </span>
      </header>
      <div className="monitor-stage">
        <div className="monitor-frame" style={{ "--ar": ratio } as CSSProperties}>
        {frame.src ? (
          <img src={frame.src} alt={`${role === "program" ? "Programme" : "Aperçu"} : ${scene?.name ?? ""}`} draggable={false} />
        ) : (
          <div className="monitor-empty">Pas d'image</div>
        )}
        </div>
      </div>
    </section>
  );
}

export function Monitors({ onCut }: { onCut: () => void }) {
  const { state, connection } = useStudio();
  const same = state ? state.preview_scene === state.program_scene : true;
  return (
    <div className="monitors">
      <Monitor role="preview" sceneId={state?.preview_scene ?? null} />
      <div className="transition-column">
        <button
          type="button"
          className="cut-button"
          onClick={(e) => {
            e.currentTarget.blur();
            onCut();
          }}
          disabled={connection !== "connected" || !state}
          title="Passer l'aperçu à l'antenne (Espace ou Entrée)"
        >
          Couper
        </button>
        <span className="transition-hint">{same ? "Aperçu = programme" : "Espace / Entrée"}</span>
      </div>
      <Monitor role="program" sceneId={state?.program_scene ?? null} />
    </div>
  );
}
