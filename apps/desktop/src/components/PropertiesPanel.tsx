/** Propriétés de la source sélectionnée : nom, réglages, position, taille, opacité. */

import type { Canvas, Rgba, Scene, Source, SourcePatch, Transform } from "@suite-media/client";
import { useEffect, useRef, useState } from "react";
import { useStudio } from "../studio/StudioContext";
import { hexToRgba, rgbaToHex } from "./color";
import { DraftInput } from "./DraftInput";
import { KIND_LABELS } from "./SourcesPanel";

/** Limite un envoi continu (curseurs, nuancier) à une requête toutes les `ms`, la dernière valeur gagne. */
function useThrottled<T>(send: (v: T) => void, ms = 80) {
  const last = useRef(0);
  const pending = useRef<{ v: T } | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const sendRef = useRef(send);
  sendRef.current = send;
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return (v: T) => {
    pending.current = { v };
    const wait = last.current + ms - performance.now();
    if (wait <= 0) {
      last.current = performance.now();
      sendRef.current(v);
      pending.current = null;
    } else if (timer.current === undefined) {
      timer.current = window.setTimeout(() => {
        timer.current = undefined;
        last.current = performance.now();
        if (pending.current) sendRef.current(pending.current.v);
        pending.current = null;
      }, wait);
    }
  };
}

const FIELDS: Array<{ key: keyof Omit<Transform, "opacity">; label: string }> = [
  { key: "x", label: "X" },
  { key: "y", label: "Y" },
  { key: "width", label: "Largeur" },
  { key: "height", label: "Hauteur" },
];

function SourceProperties({ scene, source, canvas }: { scene: Scene; source: Source; canvas: Canvas }) {
  const { run } = useStudio();
  const t = source.transform;
  const update = (patch: SourcePatch) => void run("UpdateSource", { scene: scene.id, source: source.id, ...patch });
  const setTransform = (next: Partial<Transform>) => update({ transform: { ...t, ...next } });

  // Brouillons pendant les gestes continus, pour un retour immédiat.
  const [opacity, setOpacity] = useState<number | null>(null);
  const [color, setColor] = useState<string | null>(null);
  useEffect(() => {
    setOpacity(null);
    setColor(null);
  }, [source.id]);
  const sendOpacity = useThrottled((o: number) => setTransform({ opacity: o }));
  const sendColor = useThrottled((rgba: Rgba) => update({ kind: "color", rgba }));

  const shownOpacity = opacity ?? t.opacity;

  return (
    <div className="properties">
      <div className="prop-group">
        <label className="field">
          <span>Nom</span>
          <DraftInput value={source.name} onCommit={(v) => v.trim() && update({ name: v.trim() })} />
        </label>
        <div className="prop-meta">
          <span className="chip">{KIND_LABELS[source.kind]}</span>
          <label className="toggle">
            <input type="checkbox" checked={source.visible} onChange={(e) => update({ visible: e.target.checked })} />
            <span>Visible</span>
          </label>
        </div>
      </div>

      {source.kind === "color" && (
        <div className="prop-group">
          <h3>Couleur</h3>
          <div className="field-row">
            <input
              type="color"
              aria-label="Couleur"
              value={color ?? rgbaToHex(source.rgba)}
              onChange={(e) => {
                setColor(e.target.value);
                sendColor(hexToRgba(e.target.value, source.rgba[3]));
              }}
              onBlur={() => setColor(null)}
            />
            <label className="field grow">
              <span>Alpha de la couleur</span>
              <input
                type="range"
                min={0}
                max={255}
                value={source.rgba[3]}
                onChange={(e) => sendColor([source.rgba[0], source.rgba[1], source.rgba[2], Number(e.target.value)])}
              />
            </label>
          </div>
        </div>
      )}

      {source.kind === "image" && (
        <div className="prop-group">
          <h3>Image</h3>
          <label className="field">
            <span>Chemin du fichier</span>
            <DraftInput value={source.path} spellCheck={false} onCommit={(v) => v.trim() && update({ kind: "image", path: v.trim() })} />
          </label>
        </div>
      )}

      <div className="prop-group">
        <div className="prop-title">
          <h3>Position et taille</h3>
          <span className="muted small">
            canvas {canvas.width}×{canvas.height}
          </span>
        </div>
        <div className="grid-4">
          {FIELDS.map((f) => (
            <label key={f.key} className="field">
              <span>{f.label}</span>
              <DraftInput
                type="number"
                step={1}
                value={String(Math.round(t[f.key]))}
                onCommit={(v) => {
                  const n = Number(v);
                  if (Number.isFinite(n) && (f.key === "x" || f.key === "y" || n > 0)) setTransform({ [f.key]: n });
                }}
              />
            </label>
          ))}
        </div>
        <div className="button-row">
          <button type="button" className="button small" onClick={() => setTransform({ x: 0, y: 0, width: canvas.width, height: canvas.height })}>
            Plein cadre
          </button>
          <button
            type="button"
            className="button small"
            onClick={() => setTransform({ x: Math.round((canvas.width - t.width) / 2), y: Math.round((canvas.height - t.height) / 2) })}
          >
            Centrer
          </button>
          <button
            type="button"
            className="button small"
            title="Incrustation en haut à droite, un tiers de la largeur"
            onClick={() => {
              const w = Math.round(canvas.width / 3);
              const h = Math.round((w * canvas.height) / canvas.width);
              const m = Math.round(canvas.width / 32);
              setTransform({ x: canvas.width - w - m, y: m, width: w, height: h });
            }}
          >
            Incrustation
          </button>
        </div>
      </div>

      <div className="prop-group">
        <div className="prop-title">
          <h3>Opacité</h3>
          <span className="value">{Math.round(shownOpacity * 100)} %</span>
        </div>
        <input
          type="range"
          aria-label="Opacité"
          min={0}
          max={100}
          value={Math.round(shownOpacity * 100)}
          onChange={(e) => {
            const o = Number(e.target.value) / 100;
            setOpacity(o);
            sendOpacity(o);
          }}
          onPointerUp={() => setOpacity(null)}
          onBlur={() => setOpacity(null)}
        />
      </div>
    </div>
  );
}

export function PropertiesPanel({ scene, sourceId }: { scene: Scene | undefined; sourceId: string | null }) {
  const { state } = useStudio();
  const source = scene?.sources.find((s) => s.id === sourceId);
  return (
    <section className="panel">
      <header className="panel-header">
        <h2>Propriétés</h2>
        {source && <span className="panel-sub">{source.name}</span>}
      </header>
      {scene && source && state ? (
        <SourceProperties key={source.id} scene={scene} source={source} canvas={state.canvas} />
      ) : (
        <div className="empty">
          <p>Sélectionne une source pour régler sa position, sa taille et son opacité.</p>
        </div>
      )}
    </section>
  );
}
