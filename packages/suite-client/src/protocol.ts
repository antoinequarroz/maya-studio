/**
 * Enveloppe du protocole Suite Media v1, commune à toutes les apps de la suite.
 * Miroir TypeScript du crate Rust `suite-protocol` (voir docs/protocole-suite.md).
 */

/** Version majeure du protocole parlée par ce client. */
export const PROTOCOL_VERSION = 1;

/** Ports WebSocket par défaut de chaque app de la suite. */
export const DEFAULT_PORTS: Readonly<Record<string, number>> = Object.freeze({
  studio: 4460,
  nino: 4461,
});

/** Type de service mDNS / Bonjour pour la découverte sur le réseau local. */
export const MDNS_SERVICE = "_suitemedia._tcp.local.";

/** Requêtes que toute app de la suite sait traiter (domaine `core`). */
export const CORE_REQUESTS = ["GetAppInfo", "GetRequestList", "Ping"] as const;

/** Identité d'une app de la suite. */
export interface AppInfo {
  /** Identifiant stable en minuscules : `studio`, `nino`, `cam-ios`… */
  id: string;
  /** Nom affiché. */
  name: string;
  /** Version de l'app (semver). */
  version: string;
}

/** Corps d'erreur d'une réponse. `code` est stable, `message` est pour les humains. */
export interface ErrorBody {
  code: string;
  message: string;
}

export interface HelloMessage {
  op: "hello";
  app: AppInfo;
  protocol: number;
  auth_required: boolean;
  capabilities: string[];
}

export interface IdentifiedMessage {
  op: "identified";
  client_id: string;
}

export interface ResponseMessage {
  op: "response";
  id: string;
  ok: boolean;
  data?: unknown;
  error?: ErrorBody;
}

export interface EventMessage {
  op: "event";
  app: string;
  type: string;
  data: unknown;
}

/** Messages envoyés par le serveur. */
export type ServerMessage = HelloMessage | IdentifiedMessage | ResponseMessage | EventMessage;

export interface IdentifyMessage {
  op: "identify";
  token?: string;
  name?: string;
  app?: AppInfo;
  /** Recevoir les événements (vrai par défaut côté serveur). */
  events?: boolean;
}

export interface RequestMessage {
  op: "request";
  id: string;
  type: string;
  data?: unknown;
}

/** Messages envoyés par le client. */
export type ClientMessage = IdentifyMessage | RequestMessage;

/** Codes d'erreur communs du protocole, renvoyés par le serveur. */
export const ProtocolErrorCode = {
  InvalidMessage: "invalid_message",
  NotIdentified: "not_identified",
  AuthFailed: "auth_failed",
  UnknownRequest: "unknown_request",
  InvalidData: "invalid_data",
} as const;

/** Décode un message serveur. Renvoie `null` si le message n'est pas reconnu (il est alors ignoré). */
export function parseServerMessage(raw: unknown): ServerMessage | null {
  if (typeof raw !== "string") return null;
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof value !== "object" || value === null) return null;
  const op = (value as { op?: unknown }).op;
  switch (op) {
    case "hello":
    case "identified":
    case "response":
    case "event":
      return value as ServerMessage;
    default:
      // Un client ignore ce qu'il ne connaît pas (compatibilité ascendante).
      return null;
  }
}
