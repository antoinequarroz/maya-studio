/**
 * Mixeur audio : une tranche par source sonore de la scène à l'antenne, avec volume
 * (0 à 200 %), coupure et VU-mètre stéréo (crête et RMS en dBFS, `InputLevels`).
 *
 * Les niveaux arrivent environ 10 fois par seconde : ils ne passent pas par l'état
 * React, chaque VU-mètre les lit dans un magasin partagé et se redessine en
 * `requestAnimationFrame` (montée immédiate, descente douce, crête maintenue).
 */

import { findScene, hasAudio, type Source, type SourceLevels, type StudioClient } from "@suite-media/client";
import { createContext, type CSSProperties, useContext, useEffect, useMemo, useRef, useState } from "react";
import {
  type ChannelDisplay,
  dbToFraction,
  formatGainDb,
  loudest,
  METER_FLOOR_DB,
  METER_HOT_DB,
  METER_TICKS_DB,
  METER_WARN_DB,
  percentToVolume,
  silentChannel,
  stepChannel,
  volumeToPercent,
} from "../audio/meter";
import { useStudio } from "../studio/StudioContext";
import { useThrottled } from "../useThrottled";
import { IconSpeaker, IconSpeakerOff, IconWave } from "./Icons";
import { KindIcon, SourceStatusBadge } from "./SourceBits";

/** Au-delà, les derniers niveaux reçus sont considérés comme périmés (source silencieuse). */
const STALE_MS = 400;

interface LevelsStore {
  get: (sourceId: string) => (SourceLevels & { at: number }) | undefined;
}

/** Garde les derniers niveaux reçus par source, sans provoquer de rendu React. */
function useLevelsStore(client: StudioClient): LevelsStore {
  const latest = useRef(new Map<string, SourceLevels & { at: number }>());
  useEffect(() => {
    const map = latest.current;
    const off = client.subscribe("InputLevels", (data) => {
      const at = performance.now();
      for (const s of data.sources) map.set(s.source_id, { ...s, at });
    });
    return () => {
      off();
      map.clear();
    };
  }, [client]);
  return useMemo(() => ({ get: (id) => latest.current.get(id) }), []);
}

const LevelsCtx = createContext<LevelsStore>({ get: () => undefined });

/** Fond dégradé des barres : vert, jaune à -18 dBFS, rouge à -6 dBFS. */
const METER_GRADIENT = (() => {
  const warn = dbToFraction(METER_WARN_DB) * 100;
  const hot = dbToFraction(METER_HOT_DB) * 100;
  return `linear-gradient(90deg, #1f9d55 0%, #34d058 ${warn - 0.01}%, #e3b341 ${warn}%, #f0a020 ${hot - 0.01}%, #f24b4f ${hot}%, #ff6166 100%)`;
})();

/** VU-mètre stéréo d'une source ; deux barres (gauche, droite) et une échelle. */
function Meter({ sourceId, dimmed }: { sourceId: string; dimmed: boolean }) {
  const store = useContext(LevelsCtx);
  const bars = useRef<Array<{ peak: HTMLDivElement | null; rms: HTMLDivElement | null; hold: HTMLDivElement | null }>>([
    { peak: null, rms: null, hold: null },
    { peak: null, rms: null, hold: null },
  ]);
  const root = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let raf = 0;
    let last = performance.now();
    let shown: ChannelDisplay[] = [silentChannel(), silentChannel()];
    const clip = (f: number) => `inset(0 ${((1 - f) * 100).toFixed(2)}% 0 0)`;
    const frame = (now: number) => {
      const dt = Math.min(100, now - last);
      last = now;
      const levels = store.get(sourceId);
      const fresh = levels && now - levels.at < STALE_MS;
      shown = shown.map((ch, i) => {
        // Source mono : le canal unique alimente les deux barres.
        const idx = (n: number[] | undefined) => (n && n.length > 0 ? (n[i] ?? n[0]!) : METER_FLOOR_DB);
        const target = fresh
          ? { rms: idx(levels.rms_db), peak: idx(levels.peak_db) }
          : { rms: METER_FLOOR_DB, peak: METER_FLOOR_DB };
        return stepChannel(ch, target, dt, now);
      });
      shown.forEach((ch, i) => {
        const b = bars.current[i]!;
        if (b.peak) b.peak.style.clipPath = clip(dbToFraction(ch.peak));
        if (b.rms) b.rms.style.clipPath = clip(dbToFraction(ch.rms));
        if (b.hold) {
          const f = dbToFraction(ch.hold);
          b.hold.style.left = `calc(${(f * 100).toFixed(2)}% - 2px)`;
          b.hold.style.opacity = f > 0 ? "1" : "0";
          b.hold.classList.toggle("hot", ch.hold >= METER_HOT_DB);
        }
      });
      if (root.current) {
        const max = Math.max(...shown.map((c) => c.hold));
        root.current.dataset.level = max > METER_FLOOR_DB ? "signal" : "silence";
        root.current.title = fresh
          ? `Crête ${loudest(levels.peak_db).toFixed(1).replace(".", ",")} dBFS · RMS ${loudest(levels.rms_db).toFixed(1).replace(".", ",")} dBFS`
          : "Aucun signal";
      }
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, [store, sourceId]);

  return (
    <div className={`meter${dimmed ? " dimmed" : ""}`} ref={root} data-level="silence" role="meter" aria-label="Niveau audio">
      {[0, 1].map((i) => (
        <div key={i} className="meter-bar" aria-hidden="true">
          <div className="meter-fill peak" style={{ background: METER_GRADIENT }} ref={(el) => void (bars.current[i]!.peak = el)} />
          <div className="meter-fill rms" style={{ background: METER_GRADIENT }} ref={(el) => void (bars.current[i]!.rms = el)} />
          <div className="meter-hold" ref={(el) => void (bars.current[i]!.hold = el)} />
        </div>
      ))}
      <div className="meter-scale" aria-hidden="true">
        {METER_TICKS_DB.map((db) => (
          <span key={db} style={{ left: `${dbToFraction(db) * 100}%` }}>
            {db}
          </span>
        ))}
      </div>
    </div>
  );
}

/** Une tranche du mixeur. */
function Strip({ sceneId, source }: { sceneId: string; source: Source }) {
  const { run } = useStudio();
  const [draft, setDraft] = useState<number | null>(null);
  const sendVolume = useThrottled((volume: number) => void run("SetSourceVolume", { scene: sceneId, source: source.id, volume }));
  const percent = draft ?? volumeToPercent(source.volume);
  const muted = !!source.muted;
  const setPercent = (p: number) => {
    setDraft(p);
    sendVolume(percentToVolume(p));
  };

  return (
    <li className={`mixer-strip${muted ? " muted" : ""}${source.visible ? "" : " hidden-source"}`}>
      <div className="strip-head">
        <span className="kind-icon">
          <KindIcon source={source} />
        </span>
        <span className="strip-name" title={source.name}>
          {source.name}
        </span>
        <SourceStatusBadge source={source} compact />
        <span className="strip-volume" title={formatGainDb(percentToVolume(percent))}>
          {percent} %
        </span>
        <button
          type="button"
          className={`icon-button mute-button${muted ? " on" : ""}`}
          aria-pressed={muted}
          title={muted ? "Rétablir le son" : "Couper le son"}
          onClick={() => void run("SetSourceMuted", { scene: sceneId, source: source.id, muted: !muted })}
        >
          {muted ? <IconSpeakerOff /> : <IconSpeaker />}
        </button>
      </div>
      <Meter sourceId={source.id} dimmed={muted || !source.visible} />
      <input
        type="range"
        className="volume-slider"
        aria-label={`Volume de ${source.name}`}
        title="Double-clic : 100 %"
        min={0}
        max={200}
        step={1}
        value={percent}
        style={{ "--fill": `${percent / 2}%` } as CSSProperties}
        onChange={(e) => setPercent(Number(e.target.value))}
        onDoubleClick={() => setPercent(100)}
        onPointerUp={() => setDraft(null)}
        onBlur={() => setDraft(null)}
      />
    </li>
  );
}

export function AudioMixer() {
  const { client, state } = useStudio();
  const store = useLevelsStore(client);
  const program = state ? findScene(state, state.program_scene) : undefined;
  const sources = useMemo(() => [...(program?.sources ?? [])].reverse().filter((s) => hasAudio(s.kind)), [program]);

  return (
    <section className="panel">
      <header className="panel-header">
        <h2>Audio</h2>
        {program && (
          <span className="panel-sub" title={`Mix de la scène à l'antenne : ${program.name}`}>
            {program.name}
          </span>
        )}
      </header>
      <LevelsCtx.Provider value={store}>
        {program && sources.length > 0 ? (
          <ul className="mixer">
            {sources.map((s) => (
              <Strip key={s.id} sceneId={program.id} source={s} />
            ))}
          </ul>
        ) : (
          <div className="empty">
            <IconWave size={22} />
            <p>Aucune source audio dans la scène à l'antenne.</p>
            <p className="muted small">Micro, fichier vidéo ou flux SRT : ajoute-les à la scène, puis coupe pour les entendre.</p>
          </div>
        )}
      </LevelsCtx.Provider>
    </section>
  );
}
