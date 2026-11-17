/** Écran principal : mode studio, scènes, sources, propriétés, sorties. */

import { findScene, StudioClient, isSuiteError } from "@suite-media/client";
import { type FormEvent, useEffect, useMemo, useState } from "react";
import { lastUrl, rememberUrl, resolveStudioTarget, type StudioTarget } from "./connection";
import { ConnectionBanner, StatusBar, Toasts, TopBar } from "./components/Chrome";
import { Monitors } from "./components/Monitors";
import { OutputsPanel } from "./components/OutputsPanel";
import { PropertiesPanel } from "./components/PropertiesPanel";
import { ScenesPanel } from "./components/ScenesPanel";
import { SourcesPanel } from "./components/SourcesPanel";
import { StudioProvider, useStudio } from "./studio/StudioContext";

const CLIENT_APP = { id: "studio-ui", name: "Maya Studio (interface)", version: "0.1.0" };

function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

function Workspace() {
  const { state, run } = useStudio();
  // La scène éditée est la scène d'aperçu : on prépare en aperçu, on coupe à l'antenne.
  const scene = state ? findScene(state, state.preview_scene) : undefined;
  const [selectedSource, setSelectedSource] = useState<string | null>(null);

  // Sélection : on garde la source si elle existe toujours dans la scène, sinon la plus haute.
  useEffect(() => {
    if (!scene) return;
    if (selectedSource && scene.sources.some((s) => s.id === selectedSource)) return;
    setSelectedSource(scene.sources.at(-1)?.id ?? null);
  }, [scene, selectedSource]);

  const cut = () => void run("Cut");

  // Raccourcis : Espace / Entrée = couper ; 1 à 9 = scène en aperçu.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
      if (isEditable(e.target) || document.querySelector(".modal-backdrop")) return;
      if (e.key === " " || e.key === "Enter") {
        e.preventDefault();
        cut();
      } else if (/^[1-9]$/.test(e.key) && state) {
        const target = state.scenes[Number(e.key) - 1];
        if (target) {
          e.preventDefault();
          void run("SetPreviewScene", { scene: target.id });
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <div className="app">
      <TopBar />
      <ConnectionBanner />
      <main className="workspace">
        <Monitors onCut={cut} />
        <div className="docks">
          <ScenesPanel />
          <SourcesPanel scene={scene} selectedId={selectedSource} onSelect={setSelectedSource} />
          <PropertiesPanel scene={scene} sourceId={selectedSource} />
          <OutputsPanel />
        </div>
      </main>
      <StatusBar />
      <Toasts />
    </div>
  );
}

function ConnectScreen({ error, onConnect }: { error: string | null; onConnect: (t: StudioTarget) => void }) {
  const [url, setUrl] = useState(lastUrl);
  const [token, setToken] = useState("");
  const submit = (e: FormEvent) => {
    e.preventDefault();
    rememberUrl(url.trim());
    onConnect({ url: url.trim(), token: token || undefined, origin: "manual" });
  };
  return (
    <div className="connect-screen">
      <form className="connect-card" onSubmit={submit}>
        <div className="brand large">
          <span className="brand-mark" aria-hidden="true" />
          <span className="brand-name">Maya Studio</span>
        </div>
        <p className="muted">Se connecter à un moteur de studio (protocole Suite Media).</p>
        {error && <div className="form-error">{error}</div>}
        <label className="field">
          <span>Adresse WebSocket</span>
          <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="ws://127.0.0.1:4460" spellCheck={false} />
        </label>
        <label className="field">
          <span>Jeton</span>
          <input type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" />
        </label>
        <button type="submit" className="button primary block" disabled={!/^wss?:\/\//.test(url.trim())}>
          Se connecter
        </button>
      </form>
    </div>
  );
}

function Connected({ target, onFatal }: { target: StudioTarget; onFatal: (message: string) => void }) {
  const client = useMemo(
    () =>
      new StudioClient({
        url: target.url,
        token: target.token,
        name: "Interface Maya Studio",
        app: CLIENT_APP,
        // Le moteur lancé par Tauri met un instant à démarrer : on retente vite au début.
        reconnect: { initialDelayMs: 250, maxDelayMs: 5000 },
      }),
    [target],
  );
  useEffect(() => {
    client.connect().catch((e: unknown) => {
      if (isSuiteError(e, "auth_failed")) onFatal("Jeton refusé par le studio.");
      else if (isSuiteError(e, "protocol_mismatch")) onFatal(`Studio incompatible : ${e.message}`);
      else if (!isSuiteError(e, "not_connected")) onFatal(`Connexion impossible : ${String((e as Error).message ?? e)}`);
    });
    return () => client.close();
  }, [client, onFatal]);
  return (
    <StudioProvider client={client} url={target.url}>
      <Workspace />
    </StudioProvider>
  );
}

export function App() {
  const [target, setTarget] = useState<StudioTarget | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    resolveStudioTarget()
      .then(setTarget)
      .catch((e: unknown) => {
        setError(`Moteur indisponible : ${String(e)}`);
        setTarget(null);
      });
  }, []);

  const onFatal = useMemo(
    () => (message: string) => {
      setError(message);
      setTarget(null);
    },
    [],
  );

  if (target === undefined) return <div className="connect-screen" />;
  if (target === null) {
    return (
      <ConnectScreen
        error={error}
        onConnect={(t) => {
          setError(null);
          setTarget(t);
        }}
      />
    );
  }
  return <Connected target={target} onFatal={onFatal} />;
}
