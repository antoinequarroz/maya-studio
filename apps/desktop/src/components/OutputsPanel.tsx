/** Sorties : enregistrement et direct (RTMP / SRT), avec état en temps réel. */

import type { OutputState } from "@suite-media/client";
import { useState } from "react";
import { useStudio } from "../studio/StudioContext";
import { IconBroadcast, IconRecord, IconStop } from "./Icons";

const STREAM_URL_KEY = "maya-studio.stream-url";

export function formatDuration(seconds: number | undefined): string {
  const s = Math.max(0, Math.floor(seconds ?? 0));
  const hh = String(Math.floor(s / 3600)).padStart(2, "0");
  const mm = String(Math.floor((s % 3600) / 60)).padStart(2, "0");
  const ss = String(s % 60).padStart(2, "0");
  return `${hh}:${mm}:${ss}`;
}

/** Assemble l'URL de diffusion : `rtmp://serveur/app` + `/clé`. Une URL SRT est prise telle quelle. */
export function buildStreamUrl(server: string, key: string): string {
  const base = server.trim();
  const k = key.trim();
  if (!k || base.startsWith("srt://")) return base;
  return `${base.replace(/\/+$/, "")}/${k}`;
}

/** Durée à droite du titre, ou « Inactif ». */
function OutputClock({ state }: { state: OutputState | undefined }) {
  if (!state?.active) return <span className="output-stats idle">Inactif</span>;
  return (
    <span className="output-stats">
      <span className="timecode">{formatDuration(state.duration_s)}</span>
    </span>
  );
}

/** Images envoyées et destination, sous le titre, pendant que la sortie tourne. */
function OutputDetails({ state, target }: { state: OutputState | undefined; target: string | undefined }) {
  if (!state?.active) return null;
  return (
    <div className="output-details">
      <span className="stat">
        <span className="muted">Images</span> {state.frames?.toLocaleString("fr-FR") ?? 0}
      </span>
      <span className="output-target" title={target}>
        {target || "—"}
      </span>
    </div>
  );
}

function loadStreamServer(): string {
  try {
    return localStorage.getItem(STREAM_URL_KEY) ?? "";
  } catch {
    return "";
  }
}

export function OutputsPanel() {
  const { outputs, run, connection } = useStudio();
  const [busy, setBusy] = useState<"record" | "stream" | null>(null);
  const [server, setServer] = useState(loadStreamServer);
  const [key, setKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const recording = !!outputs?.record.active;
  const streaming = !!outputs?.stream.active;
  const offline = connection !== "connected";

  const toggleRecord = async () => {
    setBusy("record");
    await run(recording ? "StopRecording" : "StartRecording");
    setBusy(null);
  };

  const toggleStream = async () => {
    setBusy("stream");
    if (streaming) {
      await run("StopStream");
    } else {
      try {
        localStorage.setItem(STREAM_URL_KEY, server.trim());
      } catch {
        // stockage indisponible
      }
      // La clé n'est jamais enregistrée localement.
      await run("StartStream", { url: buildStreamUrl(server, key) });
    }
    setBusy(null);
  };

  const streamValid = /^(rtmps?|srt):\/\/.+/.test(server.trim());

  return (
    <section className="panel">
      <header className="panel-header">
        <h2>Sorties</h2>
      </header>
      <div className="outputs">
        <div className={`output-card${recording ? " active rec" : ""}`}>
          <div className="output-title">
            <span className={`dot${recording ? " on" : ""}`} />
            <h3>Enregistrement</h3>
            {recording && <span className="badge-rec">REC</span>}
            <OutputClock state={outputs?.record} />
          </div>
          <OutputDetails state={outputs?.record} target={outputs?.record.target} />
          <button
            type="button"
            className={`button block ${recording ? "danger" : "rec"}`}
            onClick={toggleRecord}
            disabled={offline || busy === "record"}
          >
            {recording ? <IconStop /> : <IconRecord />}
            {recording ? "Arrêter l'enregistrement" : "Démarrer l'enregistrement"}
          </button>
        </div>

        <div className={`output-card${streaming ? " active live" : ""}`}>
          <div className="output-title">
            <span className={`dot${streaming ? " on" : ""}`} />
            <h3>Direct</h3>
            {streaming && <span className="badge-live">EN DIRECT</span>}
            <OutputClock state={outputs?.stream} />
          </div>
          {streaming ? (
            // L'URL complète contient la clé : on n'affiche que le serveur.
            <OutputDetails state={outputs?.stream} target={server.trim()} />
          ) : (
            <div className="form compact">
              <label className="field">
                <span>Serveur (URL RTMP ou SRT)</span>
                <input
                  value={server}
                  placeholder="rtmp://live.exemple.com/app"
                  spellCheck={false}
                  onChange={(e) => setServer(e.target.value)}
                />
              </label>
              <label className="field">
                <span>Clé de diffusion</span>
                <div className="input-with-button">
                  <input
                    type={showKey ? "text" : "password"}
                    value={key}
                    placeholder="xxxx-xxxx-xxxx"
                    spellCheck={false}
                    autoComplete="off"
                    onChange={(e) => setKey(e.target.value)}
                  />
                  <button type="button" className="button small ghost" onClick={() => setShowKey((v) => !v)}>
                    {showKey ? "Masquer" : "Afficher"}
                  </button>
                </div>
              </label>
            </div>
          )}
          <button
            type="button"
            className={`button block ${streaming ? "danger" : "live"}`}
            onClick={toggleStream}
            disabled={offline || busy === "stream" || (!streaming && !streamValid)}
            title={!streaming && !streamValid ? "Renseigne une URL rtmp://, rtmps:// ou srt://" : undefined}
          >
            {streaming ? <IconStop /> : <IconBroadcast />}
            {streaming ? "Arrêter le direct" : "Démarrer le direct"}
          </button>
        </div>
      </div>
    </section>
  );
}
