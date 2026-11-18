/** Ajout d'une source : couleur, image, mire, caméra, écran, entrée audio, fichier vidéo, flux SRT. */

import type { InputDevice, SourceKind, SourceKindName } from "@suite-media/client";
import { type FormEvent, useState } from "react";
import { baseName } from "../sourceKinds";
import { useStudio } from "../studio/StudioContext";
import { hexToRgba } from "./color";
import { Modal } from "./Dialog";
import { IconColor, IconRefresh } from "./Icons";
import { KIND_ICONS, useInputDevices } from "./SourceBits";

type AddableKind = Exclude<SourceKindName, "window_capture">;

const KINDS: Array<{ kind: AddableKind; label: string; hint: string }> = [
  { kind: "color", label: "Couleur", hint: "Aplat de couleur" },
  { kind: "image", label: "Image", hint: "PNG ou JPEG" },
  { kind: "test_pattern", label: "Mire", hint: "Mire animée" },
  { kind: "camera", label: "Caméra", hint: "Webcam, acquisition" },
  { kind: "display_capture", label: "Écran", hint: "Capture d'écran" },
  { kind: "audio_input", label: "Micro / entrée audio", hint: "Micro, interface" },
  { kind: "media_file", label: "Fichier vidéo", hint: "MP4, MKV, MOV…" },
  { kind: "srt_input", label: "Flux SRT", hint: "srt:// entrant" },
];

const DEFAULT_NAMES: Record<AddableKind, string> = {
  color: "Couleur",
  image: "Image",
  test_pattern: "Mire",
  camera: "Caméra",
  display_capture: "Écran",
  audio_input: "Micro",
  media_file: "Vidéo",
  srt_input: "SRT",
};

export const SRT_PLACEHOLDER = "srt://0.0.0.0:9000?mode=listener";

/** Premier nom libre : « Caméra », « Caméra 2 »… */
export function uniqueName(base: string, existing: readonly string[]): string {
  if (!existing.includes(base)) return base;
  for (let i = 2; ; i++) if (!existing.includes(`${base} ${i}`)) return `${base} ${i}`;
}

/** Choix d'un périphérique : « par défaut » puis la liste du moteur. */
export function DevicePicker({
  label,
  devices,
  value,
  loading,
  onChange,
  onReload,
  emptyText,
}: {
  label: string;
  devices: InputDevice[] | undefined;
  value: string;
  loading: boolean;
  onChange: (id: string) => void;
  onReload: () => void;
  emptyText: string;
}) {
  const selected = devices?.find((d) => d.id === value);
  // Périphérique enregistré mais absent de la liste (débranché) : gardé visible.
  const missing = value !== "" && devices !== null && devices !== undefined && !selected;
  return (
    <div className="field">
      <span>{label}</span>
      <div className="input-with-button">
        <select value={value} onChange={(e) => onChange(e.target.value)} aria-label={label}>
          <option value="">Par défaut du système</option>
          {devices?.map((d) => (
            <option key={d.id} value={d.id}>
              {d.name}
            </option>
          ))}
          {missing && <option value={value}>{value} (absent)</option>}
        </select>
        <button type="button" className="icon-button bordered" title="Actualiser la liste" onClick={onReload} disabled={loading}>
          <IconRefresh />
        </button>
      </div>
      <small className="muted field-hint">
        {loading && !devices
          ? "Recherche des périphériques…"
          : devices && devices.length === 0
            ? emptyText
            : (selected?.caps_summary ?? `${devices?.length ?? 0} périphérique(s) détecté(s) par le moteur.`)}
      </small>
    </div>
  );
}

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
  const { devices, loading, reload } = useInputDevices();
  const [kind, setKind] = useState<AddableKind>("color");
  const [name, setName] = useState("");
  const [color, setColor] = useState("#1e3a8a");
  const [alpha, setAlpha] = useState(255);
  const [path, setPath] = useState("");
  const [mediaPath, setMediaPath] = useState("");
  const [loop, setLoop] = useState(true);
  const [srtUrl, setSrtUrl] = useState(SRT_PLACEHOLDER);
  const [videoDevice, setVideoDevice] = useState("");
  const [audioDevice, setAudioDevice] = useState("");
  const [busy, setBusy] = useState(false);

  // Nom par défaut : le périphérique choisi (« Scarlett 2i2 »), le fichier, sinon le type.
  const deviceLabel =
    kind === "camera"
      ? devices?.video.find((d) => d.id === videoDevice)?.name
      : kind === "audio_input"
        ? devices?.audio.find((d) => d.id === audioDevice)?.name
        : undefined;
  // Fichier vidéo : son nom sans extension (« clip » pour /chemin/clip.mp4).
  const fileLabel = kind === "media_file" && mediaPath.trim() ? baseName(mediaPath.trim()).replace(/\.[^.]+$/, "") : undefined;
  const defaultName = uniqueName(deviceLabel ?? fileLabel ?? DEFAULT_NAMES[kind], existingNames);

  const valid =
    kind === "image"
      ? path.trim() !== ""
      : kind === "media_file"
        ? mediaPath.trim() !== ""
        : kind === "srt_input"
          ? /^srt:\/\/[^/?#\s]+/.test(srtUrl.trim())
          : true;

  const settings = (): SourceKind => {
    switch (kind) {
      case "color":
        return { kind, rgba: hexToRgba(color, alpha) };
      case "image":
        return { kind, path: path.trim() };
      case "camera":
        return videoDevice ? { kind, device_id: videoDevice } : { kind };
      case "audio_input":
        return audioDevice ? { kind, device_id: audioDevice } : { kind };
      case "display_capture":
        return { kind };
      case "media_file":
        return { kind, path: mediaPath.trim(), loop };
      case "srt_input":
        return { kind, url: srtUrl.trim() };
      default:
        return { kind: "test_pattern" };
    }
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!valid || busy) return;
    setBusy(true);
    const r = await run("AddSource", { scene: sceneId, name: name.trim() || defaultName, ...settings() });
    setBusy(false);
    if (r) onAdded(r.source.id);
  };

  return (
    <Modal title="Ajouter une source" onClose={onClose} width={620}>
      <form className="form" onSubmit={submit}>
        <div className="kind-picker" role="radiogroup" aria-label="Type de source">
          {KINDS.map((k) => {
            const Icon = k.kind === "color" ? IconColor : KIND_ICONS[k.kind];
            return (
              <button
                key={k.kind}
                type="button"
                role="radio"
                aria-checked={kind === k.kind}
                className={`kind-card${kind === k.kind ? " selected" : ""}`}
                onClick={() => setKind(k.kind)}
              >
                <Icon size={22} />
                <strong>{k.label}</strong>
                <small>{k.hint}</small>
              </button>
            );
          })}
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
            <input value={path} placeholder="/chemin/vers/logo.png" onChange={(e) => setPath(e.target.value)} spellCheck={false} />
          </label>
        )}
        {kind === "test_pattern" && <p className="muted small">Mire animée plein cadre, utile pour vérifier qu'une sortie est vivante.</p>}
        {kind === "camera" && (
          <DevicePicker
            label="Caméra"
            devices={devices?.video}
            value={videoDevice}
            loading={loading}
            onChange={setVideoDevice}
            onReload={reload}
            emptyText="Aucune caméra détectée par le moteur : la caméra par défaut sera essayée."
          />
        )}
        {kind === "audio_input" && (
          <DevicePicker
            label="Entrée audio"
            devices={devices?.audio}
            value={audioDevice}
            loading={loading}
            onChange={setAudioDevice}
            onReload={reload}
            emptyText="Aucune entrée audio détectée par le moteur : l'entrée par défaut sera essayée."
          />
        )}
        {kind === "display_capture" && (
          <p className="muted small">
            Capture de l'écran principal. Sous macOS, Maya Studio doit être autorisé dans Réglages Système → Confidentialité
            et sécurité → Enregistrement de l'écran.
          </p>
        )}
        {kind === "media_file" && (
          <>
            <label className="field">
              <span>Chemin du fichier sur la machine du studio</span>
              <input
                value={mediaPath}
                placeholder="/chemin/vers/clip.mp4"
                onChange={(e) => setMediaPath(e.target.value)}
                spellCheck={false}
              />
            </label>
            <label className="toggle">
              <input type="checkbox" checked={loop} onChange={(e) => setLoop(e.target.checked)} />
              <span>Lire en boucle</span>
            </label>
          </>
        )}
        {kind === "srt_input" && (
          <label className="field">
            <span>URL SRT (avec mode=listener, le studio attend l'émetteur)</span>
            <input value={srtUrl} placeholder={SRT_PLACEHOLDER} onChange={(e) => setSrtUrl(e.target.value)} spellCheck={false} />
          </label>
        )}
        <p className="muted small">
          {kind === "audio_input"
            ? "Le son part dans le mix de la scène à l'antenne ; volume et coupure se règlent dans le panneau Audio."
            : "La source est ajoutée plein cadre, au-dessus des autres. Position et taille se règlent dans les propriétés."}
        </p>
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
