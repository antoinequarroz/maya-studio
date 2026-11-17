/** Habillage de la fenêtre : barre de titre, barre d'état, notifications. */

import type { ConnectionState } from "@suite-media/client";
import { useStudio } from "../studio/StudioContext";
import { IconClose } from "./Icons";
import { formatDuration } from "./OutputsPanel";

const CONNECTION_LABELS: Record<ConnectionState, string> = {
  idle: "Non connecté",
  connecting: "Connexion…",
  connected: "Connecté",
  reconnecting: "Reconnexion…",
  closed: "Déconnecté",
};

export function TopBar() {
  const { outputs } = useStudio();
  const rec = outputs?.record;
  const live = outputs?.stream;
  return (
    <header className="topbar">
      <div className="brand">
        <span className="brand-mark" aria-hidden="true" />
        <span className="brand-name">Maya Studio</span>
      </div>
      <div className="topbar-center">
        <span className={`onair-pill${live?.active ? " on" : ""}`}>
          <span className="dot" /> Direct {live?.active ? formatDuration(live.duration_s) : ""}
        </span>
        <span className={`onair-pill rec${rec?.active ? " on" : ""}`}>
          <span className="dot" /> REC {rec?.active ? formatDuration(rec.duration_s) : ""}
        </span>
      </div>
      <div className="shortcuts" aria-label="Raccourcis clavier">
        <kbd>Espace</kbd>/<kbd>Entrée</kbd> couper <span className="sep" /> <kbd>1</kbd>–<kbd>9</kbd> aperçu
      </div>
    </header>
  );
}

export function StatusBar() {
  const { connection, info, state, url } = useStudio();
  return (
    <footer className="statusbar">
      <span className={`status-conn ${connection}`}>
        <span className="dot" />
        {CONNECTION_LABELS[connection]}
        {info && connection === "connected" && (
          <span className="muted">
            {" "}
            · {info.app.name} {info.app.version}
          </span>
        )}
      </span>
      <span className="muted mono" title="Adresse du moteur">
        {url}
      </span>
      <span className="spacer" />
      {state && (
        <>
          <span>
            <span className="muted">Canvas</span> {state.canvas.width}×{state.canvas.height}
          </span>
          <span>
            <span className="muted">Cadence</span> {state.canvas.fps} i/s
          </span>
          <span>
            <span className="muted">Scènes</span> {state.scenes.length}
          </span>
        </>
      )}
      <span className="muted" title="Les moniteurs interrogent le moteur en JPEG ; un flux d'aperçu à faible latence viendra plus tard">
        Moniteurs ~5 i/s
      </span>
    </footer>
  );
}

export function Toasts() {
  const { toasts, dismissToast } = useStudio();
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`}>
          <span>{t.text}</span>
          <button type="button" className="icon-button" onClick={() => dismissToast(t.id)} title="Fermer">
            <IconClose size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}

/** Bandeau affiché tant que la connexion n'est pas établie. */
export function ConnectionBanner() {
  const { connection } = useStudio();
  if (connection === "connected") return null;
  return (
    <div className={`connection-banner ${connection}`}>
      {connection === "closed"
        ? "Connexion au moteur perdue."
        : connection === "reconnecting"
          ? "Connexion au moteur perdue : reconnexion en cours…"
          : "Connexion au moteur du studio…"}
    </div>
  );
}
