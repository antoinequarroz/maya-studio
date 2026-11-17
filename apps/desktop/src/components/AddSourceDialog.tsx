/** Ajout d'une source : couleur, image ou mire. */

import type { SourceKind, SourceKindName } from "@suite-media/client";
import { type FormEvent, useState } from "react";
import { useStudio } from "../studio/StudioContext";
import { hexToRgba } from "./color";
import { Modal } from "./Dialog";
import { IconColor, IconImage, IconPattern } from "./Icons";

const KINDS: Array<{ kind: SourceKindName; label: string; hint: string; icon: typeof IconColor }> = [
  { kind: "color", label: "Couleur", hint: "Aplat de couleur", icon: IconColor },
  { kind: "image", label: "Image", hint: "PNG ou JPEG", icon: IconImage },
  { kind: "test_pattern", label: "Mire", hint: "Mire animée", icon: IconPattern },
];

// Types de capture : pas encore proposés par ce dialogue, noms prévus pour plus tard.
const DEFAULT_NAMES: Record<SourceKindName, string> = {
  color: "Couleur",
  image: "Image",
  test_pattern: "Mire",
  camera: "Caméra",
  display_capture: "Écran",
  window_capture: "Fenêtre",
  audio_input: "Micro",
  media_file: "Vidéo",
  srt_input: "SRT",
};

export function AddSourceDialog({
  sceneId,
  existingNames,
  onClose,
  onAdded,
}: {
  sceneId: string;
  existingNames: string[];
  onClose: () => void;
  onAdded: (sourceId: string) => void;
}) {
  const { run } = useStudio();
  const [kind, setKind] = useState<SourceKindName>("color");
  const [name, setName] = useState("");
  const [color, setColor] = useState("#1e3a8a");
  const [alpha, setAlpha] = useState(255);
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);

  const defaultName = (() => {
    const base = DEFAULT_NAMES[kind];
    if (!existingNames.includes(base)) return base;
    for (let i = 2; ; i++) if (!existingNames.includes(`${base} ${i}`)) return `${base} ${i}`;
  })();

  const valid = kind !== "image" || path.trim() !== "";

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!valid || busy) return;
    const settings: SourceKind =
      kind === "color"
        ? { kind, rgba: hexToRgba(color, alpha) }
        : kind === "image"
          ? { kind, path: path.trim() }
          : { kind: "test_pattern" };
    setBusy(true);
    const r = await run("AddSource", { scene: sceneId, name: name.trim() || defaultName, ...settings });
    setBusy(false);
    if (r) onAdded(r.source.id);
  };

  return (
    <Modal title="Ajouter une source" onClose={onClose} width={480}>
      <form className="form" onSubmit={submit}>
        <div className="kind-picker" role="radiogroup" aria-label="Type de source">
          {KINDS.map((k) => (
            <button
              key={k.kind}
              type="button"
              role="radio"
              aria-checked={kind === k.kind}
              className={`kind-card${kind === k.kind ? " selected" : ""}`}
              onClick={() => setKind(k.kind)}
            >
              <k.icon size={22} />
              <strong>{k.label}</strong>
              <small>{k.hint}</small>
            </button>
          ))}
        </div>
        <label className="field">
          <span>Nom</span>
          <input value={name} placeholder={defaultName} onChange={(e) => setName(e.target.value)} autoFocus />
        </label>
        {kind === "color" && (
          <div className="field-row">
            <label className="field">
              <span>Couleur</span>
              <input type="color" value={color} onChange={(e) => setColor(e.target.value)} />
            </label>
            <label className="field grow">
              <span>Opacité de la couleur : {Math.round((alpha / 255) * 100)} %</span>
              <input type="range" min={0} max={255} value={alpha} onChange={(e) => setAlpha(Number(e.target.value))} />
            </label>
          </div>
        )}
        {kind === "image" && (
          <label className="field">
            <span>Chemin du fichier sur la machine du studio</span>
            <input
              value={path}
              placeholder="/chemin/vers/logo.png"
              onChange={(e) => setPath(e.target.value)}
              spellCheck={false}
            />
          </label>
        )}
        {kind === "test_pattern" && <p className="muted small">Mire animée plein cadre, utile pour vérifier qu'une sortie est vivante.</p>}
        <p className="muted small">La source est ajoutée plein cadre, au-dessus des autres. Position et taille se règlent dans les propriétés.</p>
        <div className="modal-actions">
          <button type="button" className="button" onClick={onClose}>
            Annuler
          </button>
          <button type="submit" className="button primary" disabled={!valid || busy}>
            Ajouter
          </button>
        </div>
      </form>
    </Modal>
  );
}
