/** Écran principal : mode studio, scènes, sources, propriétés, sorties. */

import { findScene, StudioClient } from "@suite-media/client";
import { type FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { lastUrl, onEngineEvent, rememberUrl, resolveStudioTarget, type StudioTarget } from "./connection";
import { connectWatchingFatal } from "./fatal";
import { ConnectionBanner, StatusBar, Toasts, TopBar } from "./components/Chrome";
import { Monitors } from "./components/Monitors";
import { OutputsPanel } from "./components/OutputsPanel";
import { PropertiesPanel } from "./components/PropertiesPanel";
import { ScenesPanel } from "./components/ScenesPanel";
import { SourcesPanel } from "./components/SourcesPanel";
import { PENDING_SELECTION_MS, type PendingSelection, reconcileSelection } from "./selection";
import { isInteractiveTarget } from "./shortcuts";
import { StudioProvider, useStudio } from "./studio/StudioContext";

const CLIENT_APP = { id: "studio-ui", name: "Maya Studio (interface)", version: "0.1.0" };

function Workspace() {
  const { state, run, notify } = useStudio();
  // La scène éditée est la scène d'aperçu : on prépare en aperçu, on coupe à l'antenne.
  const scene = state ? findScene(state, state.preview_scene) : undefined;
  const [selectedSource, setSelectedSource] = useState<string | null>(null);
  // Source sélectionnée avant d'être dans la scène (réponse à AddSource reçue avant SourceAdded).
  const pendingSelection = useRef<PendingSelection | null>(null);
  const [selectionCheck, setSelectionCheck] = useState(0);
  const sceneSourceIds = useRef<string[]>([]);
  sceneSourceIds.current = scene?.sources.map((s) => s.id) ?? [];

  const selectSource = useCallback((id: string | null) => {
    if (id && !sceneSourceIds.current.includes(id)) {
      pendingSelection.current = { id, until: Date.now() + PENDING_SELECTION_MS };
      // Si l'événement n'arrive jamais, on revient à la règle normale à l'échéance.
      window.setTimeout(() => setSelectionCheck((n) => n + 1), PENDING_SELECTION_MS + 50);
    } else {
      pendingSelection.current = null;
    }
    setSelectedSource(id);
  }, []);

  // Sélection : on garde la source si elle existe toujours dans la scène (ou si son ajout
  // est en route), sinon la plus haute.
  useEffect(() => {
    if (!scene) return;
    const ids = scene.sources.map((s) => s.id);
    const next = reconcileSelection(ids, selectedSource, pendingSelection.current, Date.now());
    const pending = pendingSelection.current;
    if (pending && (ids.includes(pending.id) || next !== pending.id)) pendingSelection.current = null;
    if (next !== selectedSource) setSelectedSource(next);
  }, [scene, selectedSource, selectionCheck]);

  // Moteur lancé par l'app : arrêt inattendu et redémarrage signalés par le shell Tauri.
  useEffect(() => {
    let off: (() => void) | undefined;
    let disposed = false;
    void onEngineEvent((ev) => notify(ev.kind === "restarted" ? "info" : "error", ev.message)).then((f) => {
      if (disposed) f();
      else off = f;
    });
    return () => {
      disposed = true;
      off?.();
    };
  }, [notify]);

  const cut = () => void run("Cut");

  // Raccourcis : Espace / Entrée = couper ; 1 à 9 = scène en aperçu.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
      if (isInteractiveTarget(e.target) || document.querySelector(".modal-backdrop")) return;
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
          <SourcesPanel scene={scene} selectedId={selectedSource} onSelect={selectSource} />
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
  useEffect(() => connectWatchingFatal(client, onFatal), [client, onFatal]);
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
