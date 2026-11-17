/**
 * Contexte React du studio : un seul `StudioClient`, l'état local tenu à jour
 * uniquement par les événements, l'état des sorties et les notifications.
 */

import {
  applyStudioEvent,
  type ConnectionInfo,
  type ConnectionState,
  type OutputStatus,
  type StudioClient,
  type StudioRequests,
  type StudioRequestType,
  type StudioState,
  SuiteError,
} from "@suite-media/client";
import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";

export interface Toast {
  id: number;
  kind: "error" | "info";
  text: string;
}

export interface StudioContextValue {
  client: StudioClient;
  url: string;
  connection: ConnectionState;
  info: ConnectionInfo | null;
  state: StudioState | null;
  outputs: OutputStatus | null;
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
      default:
        return e.message.charAt(0).toUpperCase() + e.message.slice(1);
    }
  }
  return String(e);
}

export function StudioProvider({ client, url, children }: { client: StudioClient; url: string; children: ReactNode }) {
  const [connection, setConnection] = useState<ConnectionState>(client.state);
  const [info, setInfo] = useState<ConnectionInfo | null>(client.info);
  const [state, setState] = useState<StudioState | null>(null);
  const [outputs, setOutputs] = useState<OutputStatus | null>(null);
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

    const offEvents = client.subscribe("*", (data, ev) => {
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
      try {
        const snapshot = await client.call("GetStudio");
        let s = snapshot;
        for (const [type, data] of backlog) s = applyStudioEvent(s, type, data);
        backlog = [];
        snapshotLoaded = true;
        setState(s);
        void refreshOutputs();
      } catch (e) {
        notify("error", `Chargement de l'état impossible : ${describeError(e)}`);
      }
    };

    const offState = client.onState((st, inf) => {
      setConnection(st);
      setInfo(inf);
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
    () => ({ client, url, connection, info, state, outputs, toasts, dismissToast, notify, run }),
    [client, url, connection, info, state, outputs, toasts, dismissToast, notify, run],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
