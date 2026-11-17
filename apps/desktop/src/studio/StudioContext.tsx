/**
 * Contexte React du studio : un seul `StudioClient`, l'état local tenu à jour
 * uniquement par les événements, l'état des sorties et les notifications.
 */

import {
  applyStudioEvent,
  type ConnectionInfo,
  type ConnectionState,
  type EncodersInfo,
  type CaptureState,
  isCaptureKind,
  type OutputStatus,
  type Source,
  type SourceFailedData,
  type SourceStartedData,
  type StudioClient,
  type StudioRequests,
  type StudioRequestType,
  type StudioState,
  SuiteError,
} from "@suite-media/client";
import { KIND_LABELS } from "../sourceKinds";
import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";

export interface Toast {
  id: number;
  kind: "error" | "info";
  text: string;
}

/** État d'une capture, d'après `SourceStarted` / `SourceFailed`. */
export type CaptureStatus =
  | { state: "starting" }
  | { state: "running" }
  | { state: "failed"; error: string; retryInMs: number; at: number };

export interface StudioContextValue {
  client: StudioClient;
  url: string;
  connection: ConnectionState;
  info: ConnectionInfo | null;
  state: StudioState | null;
  outputs: OutputStatus | null;
  /** Encodeurs du moteur (`GetEncoders`), chargés à la connexion. */
  encoders: EncodersInfo | null;
  /**
   * État des captures par identifiant de source : instantané `GetCaptureStatus` à la
   * connexion, puis `SourceAdded` / `SourceStarted` / `SourceFailed`.
   */
  captures: Readonly<Record<string, CaptureStatus>>;
  toasts: Toast[];
  dismissToast: (id: number) => void;
  notify: (kind: Toast["kind"], text: string) => void;
  /** Envoie une requête typée ; une erreur devient une notification et renvoie `undefined`. */
  run: <K extends StudioRequestType>(
    type: K,
    data?: StudioRequests[K]["data"],
  ) => Promise<StudioRequests[K]["result"] | undefined>;
}

const Ctx = createContext<StudioContextValue | null>(null);

export function useStudio(): StudioContextValue {
  const v = useContext(Ctx);
  if (!v) throw new Error("useStudio hors de StudioProvider");
  return v;
}

const OUTPUT_EVENTS = new Set(["OutputStarted", "OutputStopped", "OutputFailed"]);
/** Événements fréquents sans effet sur l'état du studio : lus directement par le mixeur. */
const HIGH_RATE_EVENTS = new Set(["InputLevels"]);

/** Libellés français des erreurs les plus courantes (le message du serveur sert sinon). */
function describeError(e: unknown): string {
  if (e instanceof SuiteError) {
    switch (e.code) {
      case "timeout":
        return "Le studio ne répond pas (délai dépassé).";
      case "not_connected":
      case "connection_closed":
        return "Pas de connexion au studio.";
      case "scene_on_air":
        return "Impossible de supprimer la scène à l'antenne.";
      case "last_scene":
        return "Impossible de supprimer la dernière scène.";
      case "scene_name_taken":
        return e.message.charAt(0).toUpperCase() + e.message.slice(1) + ".";
      case "output_error":
        return `Échec de la sortie : ${e.message}`;
      case "invalid_volume":
        return "Volume invalide : entre 0 et 200 %.";
      default:
        return e.message.charAt(0).toUpperCase() + e.message.slice(1);
    }
  }
  return String(e);
}

function fromSnapshot(c: CaptureState): CaptureStatus {
  return c.state === "failed"
    ? { state: "failed", error: c.error, retryInMs: c.retry_in_ms, at: Date.now() }
    : { state: c.state };
}

export function StudioProvider({ client, url, children }: { client: StudioClient; url: string; children: ReactNode }) {
  const [connection, setConnection] = useState<ConnectionState>(client.state);
  const [info, setInfo] = useState<ConnectionInfo | null>(client.info);
  const [state, setState] = useState<StudioState | null>(null);
  const [outputs, setOutputs] = useState<OutputStatus | null>(null);
  const [encoders, setEncoders] = useState<EncodersInfo | null>(null);
  const [captures, setCaptures] = useState<Record<string, CaptureStatus>>({});
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextToast = useRef(1);

  const notify = useCallback((kind: Toast["kind"], text: string) => {
    const id = nextToast.current++;
    setToasts((t) => [...t.slice(-3), { id, kind, text }]);
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), kind === "error" ? 6000 : 3500);
  }, []);

  const dismissToast = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);

  const refreshOutputs = useCallback(async () => {
    try {
      setOutputs(await client.call("GetOutputStatus"));
    } catch {
      // passager : la prochaine mise à jour corrigera
    }
  }, [client]);

  // Événements : état local tenu par le réducteur, pour toutes les apps abonnées.
  useEffect(() => {
    // Les événements qui arrivent avant l'instantané initial sont mis de côté puis rejoués.
    let snapshotLoaded = false;
    let backlog: Array<[string, unknown]> = [];
    const failedSources = new Set<string>();
    // Captures mises à jour par un événement depuis la dernière demande d'instantané :
    // l'instantané, peut-être plus ancien, ne les écrase pas.
    let touched = new Set<string>();

    const offEvents = client.subscribe("*", (data, ev) => {
      if (HIGH_RATE_EVENTS.has(ev.type)) return;
      const touch = (id: string) => touched.add(id);
      if (ev.type === "SourceAdded") {
        const { source } = data as { source: Source };
        if (isCaptureKind(source.kind)) {
          touch(source.id);
          setCaptures((c) => (source.id in c ? c : { ...c, [source.id]: { state: "starting" } }));
        }
      } else if (ev.type === "SourceStarted") {
        const d = data as SourceStartedData;
        touch(d.source_id);
        failedSources.delete(d.source_id);
        setCaptures((c) => (c[d.source_id]?.state === "running" ? c : { ...c, [d.source_id]: { state: "running" } }));
      } else if (ev.type === "SourceFailed") {
        const d = data as SourceFailedData;
        touch(d.source_id);
        // Une notification au passage en échec, pas à chaque nouvel essai.
        if (!failedSources.has(d.source_id)) {
          failedSources.add(d.source_id);
          notify("error", `${KIND_LABELS[d.kind] ?? "Source"} en échec : ${d.error}`);
        }
        setCaptures((c) => ({
          ...c,
          [d.source_id]: { state: "failed", error: d.error, retryInMs: d.retry_in_ms, at: Date.now() },
        }));
      } else if (ev.type === "SourceRemoved") {
        const { source_id } = data as { source_id: string };
        setCaptures((c) => {
          if (!(source_id in c)) return c;
          const { [source_id]: _, ...rest } = c;
          return rest;
        });
        failedSources.delete(source_id);
      }
      if (!snapshotLoaded) {
        backlog.push([ev.type, data]);
      } else {
        setState((s) => (s ? applyStudioEvent(s, ev.type, data) : s));
      }
      if (OUTPUT_EVENTS.has(ev.type)) {
        void refreshOutputs();
        if (ev.type === "OutputFailed") {
          const d = data as { output: string; error: string };
          notify("error", `${d.output === "record" ? "Enregistrement" : "Direct"} interrompu : ${d.error}`);
        }
      }
    });

    const load = async () => {
      snapshotLoaded = false;
      backlog = [];
      touched = new Set();
      try {
        const snapshot = await client.call("GetStudio");
        let s = snapshot;
        for (const [type, data] of backlog) s = applyStudioEvent(s, type, data);
        backlog = [];
        snapshotLoaded = true;
        setState(s);
        void refreshOutputs();
        // Encodeurs et état des captures : informatifs, un moteur plus ancien peut ne
        // pas connaître ces requêtes.
        client.call("GetEncoders").then(setEncoders, () => setEncoders(null));
        client.call("GetCaptureStatus").then(
          ({ sources }) =>
            setCaptures((current) => {
              const next: Record<string, CaptureStatus> = {};
              for (const c of sources) {
                next[c.source_id] = touched.has(c.source_id) && current[c.source_id] ? current[c.source_id]! : fromSnapshot(c);
                if (c.state === "failed") failedSources.add(c.source_id);
              }
              return next;
            }),
          () => undefined,
        );
      } catch (e) {
        notify("error", `Chargement de l'état impossible : ${describeError(e)}`);
      }
    };

    const offState = client.onState((st, inf) => {
      setConnection(st);
      setInfo(inf);
      // Les états de capture d'avant la coupure ne sont plus sûrs.
      if (st !== "connected") setCaptures((c) => (Object.keys(c).length ? {} : c));
      // À chaque (re)connexion, on repart d'un instantané complet.
      if (st === "connected") void load();
    });
    if (client.state === "connected") void load();

    return () => {
      offEvents();
      offState();
    };
  }, [client, notify, refreshOutputs]);

  // Pendant qu'une sortie tourne, on rafraîchit durée et images chaque seconde.
  const anyActive = !!(outputs?.record.active || outputs?.stream.active);
  useEffect(() => {
    if (!anyActive || connection !== "connected") return;
    const t = window.setInterval(() => void refreshOutputs(), 1000);
    return () => window.clearInterval(t);
  }, [anyActive, connection, refreshOutputs]);

  const run = useCallback(
    async <K extends StudioRequestType>(type: K, data?: StudioRequests[K]["data"]) => {
      try {
        // `call` (et non `request`) : il complète les données optionnelles attendues par le serveur.
        const call = client.call.bind(client) as (t: string, d?: unknown) => Promise<unknown>;
        return (await call(type, data)) as StudioRequests[K]["result"];
      } catch (e) {
        notify("error", describeError(e));
        return undefined;
      }
    },
    [client, notify],
  );

  const value = useMemo<StudioContextValue>(
    () => ({ client, url, connection, info, state, outputs, encoders, captures, toasts, dismissToast, notify, run }),
    [client, url, connection, info, state, outputs, encoders, captures, toasts, dismissToast, notify, run],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
