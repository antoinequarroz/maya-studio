/** Liste des scènes : clic = aperçu, double-clic = programme. */

import { useState } from "react";
import { useStudio } from "../studio/StudioContext";
import { ConfirmDialog, PromptDialog } from "./Dialog";
import { IconPencil, IconPlus, IconTrash } from "./Icons";

type DialogState = { kind: "create" } | { kind: "rename"; id: string; name: string } | { kind: "delete"; id: string; name: string };

function nextSceneName(existing: string[]): string {
  for (let i = existing.length + 1; ; i++) {
    const name = `Scène ${i}`;
    if (!existing.includes(name)) return name;
  }
}

export function ScenesPanel() {
  const { state, run } = useStudio();
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const scenes = state?.scenes ?? [];

  return (
    <section className="panel">
      <header className="panel-header">
        <h2>Scènes</h2>
        <span className="panel-count">{scenes.length}</span>
        <div className="panel-actions">
          <button type="button" className="icon-button" title="Nouvelle scène" onClick={() => setDialog({ kind: "create" })}>
            <IconPlus />
          </button>
        </div>
      </header>
      <ul className="list" role="listbox" aria-label="Scènes">
        {scenes.map((s, i) => {
          const program = s.id === state?.program_scene;
          const preview = s.id === state?.preview_scene;
          return (
            <li
              key={s.id}
              role="option"
              aria-selected={preview}
              className={`list-item scene-item${program ? " is-program" : ""}${preview ? " is-preview" : ""}`}
              onClick={() => !preview && void run("SetPreviewScene", { scene: s.id })}
              onDoubleClick={() => void run("SetProgramScene", { scene: s.id })}
              title="Clic : aperçu · double-clic : programme"
            >
              <span className="scene-key">{i < 9 ? i + 1 : ""}</span>
              <span className="list-name">{s.name}</span>
              <span className="scene-tallies">
                {program && <span className="mini-tally program">PGM</span>}
                {preview && <span className="mini-tally preview">PVW</span>}
              </span>
              <span className="row-actions">
                <button
                  type="button"
                  className="icon-button"
                  title="Renommer"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDialog({ kind: "rename", id: s.id, name: s.name });
                  }}
                  onDoubleClick={(e) => e.stopPropagation()}
                >
                  <IconPencil />
                </button>
                <button
                  type="button"
                  className="icon-button danger"
                  title={program ? "Scène à l'antenne : suppression impossible" : "Supprimer"}
                  disabled={program}
                  onClick={(e) => {
                    e.stopPropagation();
                    setDialog({ kind: "delete", id: s.id, name: s.name });
                  }}
                  onDoubleClick={(e) => e.stopPropagation()}
                >
                  <IconTrash />
                </button>
              </span>
            </li>
          );
        })}
      </ul>

      {dialog?.kind === "create" && (
        <PromptDialog
          title="Nouvelle scène"
          label="Nom de la scène"
          initial={nextSceneName(scenes.map((s) => s.name))}
          confirmLabel="Créer"
          onClose={() => setDialog(null)}
          onSubmit={async (name) => {
            const r = await run("CreateScene", { name });
            if (r) setDialog(null);
          }}
        />
      )}
      {dialog?.kind === "rename" && (
        <PromptDialog
          title="Renommer la scène"
          label="Nouveau nom"
          initial={dialog.name}
          confirmLabel="Renommer"
          onClose={() => setDialog(null)}
          onSubmit={async (name) => {
            if (name === dialog.name) return setDialog(null);
            const r = await run("RenameScene", { scene: dialog.id, name });
            if (r) setDialog(null);
          }}
        />
      )}
      {dialog?.kind === "delete" && (
        <ConfirmDialog
          title="Supprimer la scène"
          message={
            <>
              Supprimer la scène <strong>{dialog.name}</strong> et toutes ses sources ?
            </>
          }
          confirmLabel="Supprimer"
          onClose={() => setDialog(null)}
          onConfirm={async () => {
            setDialog(null);
            await run("RemoveScene", { scene: dialog.id });
          }}
        />
      )}
    </section>
  );
}
