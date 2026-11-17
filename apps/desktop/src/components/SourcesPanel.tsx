/** Sources de la scène sélectionnée, de la plus haute (au-dessus) à la plus basse. */

import type { Scene, Source } from "@suite-media/client";
import { useState } from "react";
import { useStudio } from "../studio/StudioContext";
import { AddSourceDialog } from "./AddSourceDialog";
import { rgbaToCss } from "./color";
import { ConfirmDialog } from "./Dialog";
import { IconDown, IconEye, IconEyeOff, IconImage, IconLayers, IconPattern, IconPlus, IconTrash, IconUp } from "./Icons";

export const KIND_LABELS: Record<Source["kind"], string> = { color: "Couleur", image: "Image", test_pattern: "Mire" };

function KindIcon({ source }: { source: Source }) {
  if (source.kind === "color") return <span className="swatch" style={{ background: rgbaToCss(source.rgba) }} />;
  if (source.kind === "image") return <IconImage />;
  return <IconPattern />;
}

export function SourcesPanel({
  scene,
  selectedId,
  onSelect,
}: {
  scene: Scene | undefined;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
}) {
  const { run } = useStudio();
  const [adding, setAdding] = useState(false);
  const [deleting, setDeleting] = useState<Source | null>(null);
  const sources = scene?.sources ?? [];
  // Affichage du haut de la pile vers le bas, comme dans un logiciel de montage.
  const display = [...sources].reverse();

  const move = (s: Source, delta: number) => {
    if (!scene) return;
    const idx = sources.findIndex((x) => x.id === s.id);
    const index = idx + delta;
    if (index < 0 || index >= sources.length) return;
    void run("MoveSource", { scene: scene.id, source: s.id, index });
  };

  return (
    <section className="panel">
      <header className="panel-header">
        <h2>Sources</h2>
        {scene && <span className="panel-sub" title={scene.name}>{scene.name}</span>}
        <div className="panel-actions">
          <button type="button" className="icon-button" title="Ajouter une source" disabled={!scene} onClick={() => setAdding(true)}>
            <IconPlus />
          </button>
        </div>
      </header>
      {sources.length === 0 ? (
        <div className="empty">
          <IconLayers size={22} />
          <p>Aucune source dans cette scène.</p>
          {scene && (
            <button type="button" className="button small" onClick={() => setAdding(true)}>
              <IconPlus /> Ajouter une source
            </button>
          )}
        </div>
      ) : (
        <ul className="list" role="listbox" aria-label="Sources">
          {display.map((s, i) => (
            <li
              key={s.id}
              role="option"
              aria-selected={s.id === selectedId}
              className={`list-item source-item${s.id === selectedId ? " selected" : ""}${s.visible ? "" : " hidden-source"}`}
              onClick={() => onSelect(s.id)}
            >
              <button
                type="button"
                className="icon-button"
                title={s.visible ? "Masquer" : "Afficher"}
                onClick={(e) => {
                  e.stopPropagation();
                  void run("UpdateSource", { scene: scene!.id, source: s.id, visible: !s.visible });
                }}
              >
                {s.visible ? <IconEye /> : <IconEyeOff />}
              </button>
              <span className="kind-icon" title={KIND_LABELS[s.kind]}>
                <KindIcon source={s} />
              </span>
              <span className="list-name">{s.name}</span>
              <span className="row-actions">
                <button
                  type="button"
                  className="icon-button"
                  title="Monter"
                  disabled={i === 0}
                  onClick={(e) => {
                    e.stopPropagation();
                    move(s, +1);
                  }}
                >
                  <IconUp />
                </button>
                <button
                  type="button"
                  className="icon-button"
                  title="Descendre"
                  disabled={i === display.length - 1}
                  onClick={(e) => {
                    e.stopPropagation();
                    move(s, -1);
                  }}
                >
                  <IconDown />
                </button>
                <button
                  type="button"
                  className="icon-button danger"
                  title="Supprimer"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDeleting(s);
                  }}
                >
                  <IconTrash />
                </button>
              </span>
            </li>
          ))}
        </ul>
      )}

      {adding && scene && (
        <AddSourceDialog
          sceneId={scene.id}
          existingNames={sources.map((s) => s.name)}
          onClose={() => setAdding(false)}
          onAdded={(id) => {
            setAdding(false);
            onSelect(id);
          }}
        />
      )}
      {deleting && scene && (
        <ConfirmDialog
          title="Supprimer la source"
          message={
            <>
              Supprimer la source <strong>{deleting.name}</strong> de la scène {scene.name} ?
            </>
          }
          confirmLabel="Supprimer"
          onClose={() => setDeleting(null)}
          onConfirm={async () => {
            const s = deleting;
            setDeleting(null);
            const r = await run("RemoveSource", { scene: scene.id, source: s.id });
            if (r && s.id === selectedId) onSelect(null);
          }}
        />
      )}
    </section>
  );
}
