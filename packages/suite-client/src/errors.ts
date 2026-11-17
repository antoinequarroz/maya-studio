/**
 * Erreurs typées du client. `code` est stable et sert aux programmes :
 * soit un code renvoyé par le serveur (`scene_not_found`, `auth_failed`…),
 * soit un code propre au client (voir {@link ClientErrorCode}).
 */

/** Codes d'erreur produits par le client lui-même (jamais par le serveur). */
export const ClientErrorCode = {
  /** Pas de réponse dans le délai imparti. */
  Timeout: "timeout",
  /** Le client n'est pas connecté (jamais connecté, ou fermé). */
  NotConnected: "not_connected",
  /** La connexion a été perdue avant la réponse. */
  ConnectionClosed: "connection_closed",
  /** Le serveur parle une autre version majeure du protocole. */
  ProtocolMismatch: "protocol_mismatch",
  /** Le serveur ne propose pas le domaine nécessaire à la requête. */
  CapabilityMissing: "capability_missing",
  /** La requête a été annulée par l'appelant. */
  Aborted: "aborted",
  /** Pas de WebSocket disponible dans cet environnement. */
  NoWebSocket: "no_websocket",
} as const;

export type ClientErrorCode = (typeof ClientErrorCode)[keyof typeof ClientErrorCode];

export class SuiteError extends Error {
  /** Code stable (serveur ou client). */
  readonly code: string;
  /** Type de la requête concernée, s'il y en a une. */
  readonly requestType: string | undefined;
  /** Vrai si l'erreur vient du serveur (réponse `ok: false`). */
  readonly fromServer: boolean;

  constructor(code: string, message: string, options: { requestType?: string; fromServer?: boolean } = {}) {
    super(message);
    this.name = "SuiteError";
    this.code = code;
    this.requestType = options.requestType;
    this.fromServer = options.fromServer ?? false;
  }
}

/** Vrai si `e` est une {@link SuiteError}, éventuellement avec le code donné. */
export function isSuiteError(e: unknown, code?: string): e is SuiteError {
  return e instanceof SuiteError && (code === undefined || e.code === code);
}
