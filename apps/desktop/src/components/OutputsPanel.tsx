/** Sorties : enregistrement et direct (RTMP / SRT), avec état en temps réel. */

import type { EncodersInfo, OutputState, RecordFormat } from "@suite-media/client";
import { useState } from "react";
import { useStudio } from "../studio/StudioContext";
import { IconBroadcast, IconRecord, IconStop } from "./Icons";

const STREAM_URL_KEY = "maya-studio.stream-url";
const RECORD_FORMAT_KEY = "maya-studio.record-format";

const FORMATS: Array<{ format: RecordFormat; label: string; hint: string }> = [
  { format: "mp4", label: "MP4", hint: "MP4 : lu partout ; finalisé à l'arrêt de l'enregistrement" },
  { format: "mkv", label: "MKV", hint: "Matroska : reste lisible même après un arrêt brutal" },
];

/**
 * Encodeur à afficher : celui de la sortie active, sinon celui que le moteur choisira
 * pour la prochaine sortie (`GetEncoders`). Matériel / logiciel quand on le sait.
 */
export function describeEncoder(
  state: OutputState | undefined,
  encoders: EncodersInfo | null,
): { name: string; kind: "matériel" | "logiciel" | null } | null {
  const name = state?.active ? state.encoder : encoders?.encoders.find((e) => e.selected)?.name;
  if (!name) return null;
  const info = encoders?.encoders.find((e) => e.name === name);
  return { name, kind: info ? (info.hardware ? "matériel" : "logiciel") : null };
}

function loadRecordFormat(): RecordFormat {
  try {
    return localStorage.getItem(RECORD_FORMAT_KEY) === "mkv" ? "mkv" : "mp4";
  } catch {
    return "mp4";
  }
}

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

/**
 * Destination de diffusion affichable, sans secret : l'URL complète porte la clé.
 * - RTMP(S) : le dernier segment du chemin est la clé (`rtmp://serveur/app/clé`) ;
 * - SRT : la clé est dans la requête (`streamid`, `passphrase`…), qu'on retire entière ;
 * - dans tous les cas, identifiant et mot de passe éventuels sont retirés.
 */
export function redactStreamTarget(target: string | undefined): string {
  const m = (target ?? "").trim().match(/^([a-z][a-z0-9+.-]*):\/\/([^/?#]*)([^?#]*)/i);
  if (!m) return "";
  const scheme = m[1]!.toLowerCase();
  const host = m[2]!.replace(/^.*@/, "");
  const segments = m[3]!.split("/").filter(Boolean);
  const kept = (scheme === "rtmp" || scheme === "rtmps") && segments.length > 1 ? segments.slice(0, -1) : segments;
  return `${scheme}://${host}${kept.length ? `/${kept.join("/")}` : ""}`;
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
      {!!state.dropped_frames && (
        <span className="stat warn" title="Images perdues : l'encodeur ou le réseau ne suivait pas">
          <span className="muted">Perdues</span> {state.dropped_frames.toLocaleString("fr-FR")}
        </span>
      )}
      <span className="output-target" title={target}>
        {/* Marques LTR : avec direction rtl (ellipse à gauche), un chemin garderait sinon son « / » à la fin. */}
        {target ? `\u200e${target}\u200e` : "—"}
      </span>
    </div>
  );
}

/** Encodeur et débit : effectifs pendant la sortie, prévus sinon. */
function EncoderLine({ state, encoders }: { state: OutputState | undefined; encoders: EncodersInfo | null }) {
  const encoder = describeEncoder(state, encoders);
  if (!encoder) return null;
  const active = !!state?.active;
  return (
    <div className="encoder-line" title={active ? "Encodeur utilisé par cette sortie" : "Encodeur que le moteur utilisera"}>
      <span className="muted">{active ? "Encodeur" : "Encodeur prévu"}</span>
      <span className="mono">{encoder.name}</span>
      {encoder.kind && <span className="muted">· {encoder.kind}</span>}
      {active && state?.bitrate_kbps && <span className="muted">· {state.bitrate_kbps.toLocaleString("fr-FR")} kbit/s</span>}
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
  const { outputs, run, connection, encoders } = useStudio();
  const [format, setFormat] = useState<RecordFormat>(loadRecordFormat);
  const [busy, setBusy] = useState<"record" | "stream" | null>(null);
  const [server, setServer] = useState(loadStreamServer);
  const [key, setKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const recording = !!outputs?.record.active;
  const streaming = !!outputs?.stream.active;
  const offline = connection !== "connected";

  const toggleRecord = async () => {
    setBusy("record");
    if (recording) {
      await run("StopRecording");
    } else {
      await run("StartRecording", { format });
    }
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
          <div className="output-options">
            <span className="muted small">Format</span>
            <div className="segmented" role="radiogroup" aria-label="Format d'enregistrement">
              {FORMATS.map((f) => {
                // Pendant l'enregistrement, on montre le format réellement utilisé.
                const checked = recording ? outputs?.record.format === f.format : format === f.format;
                return (
                  <button
                    key={f.format}
                    type="button"
                    role="radio"
                    aria-checked={checked}
                    className={checked ? "selected" : undefined}
                    title={f.hint}
                    disabled={recording}
                    onClick={() => {
                      setFormat(f.format);
                      try {
                        localStorage.setItem(RECORD_FORMAT_KEY, f.format);
                      } catch {
                        // stockage indisponible
                      }
                    }}
                  >
                    {f.label}
                  </button>
                );
              })}
            </div>
          </div>
          <EncoderLine state={outputs?.record} encoders={encoders} />
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
            // Destination réelle du moteur (le direct a pu être lancé par une autre app),
            // sans la clé de diffusion.
            <>
              <OutputDetails state={outputs?.stream} target={redactStreamTarget(outputs?.stream.target)} />
              <EncoderLine state={outputs?.stream} encoders={encoders} />
            </>
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
