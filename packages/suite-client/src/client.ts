/**
 * Client générique du protocole Suite Media v1.
 *
 * Fonctionne partout où il y a un WebSocket « standard » : navigateur, Tauri,
 * Node 22+ (WebSocket global), React Native. Aucune dépendance.
 */

import { backoffDelay, type BackoffOptions, DEFAULT_BACKOFF } from "./backoff.js";
import { ClientErrorCode, SuiteError } from "./errors.js";
import {
  type AppInfo,
  type ClientMessage,
  type EventMessage,
  type HelloMessage,
  parseServerMessage,
  PROTOCOL_VERSION,
  ProtocolErrorCode,
  type ResponseMessage,
} from "./protocol.js";

/** Le strict nécessaire d'un WebSocket ; compatible navigateur, Node 22 et React Native. */
export interface WebSocketLike {
  readonly readyState: number;
  onopen: ((ev: never) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onclose: ((ev: { code?: number; reason?: string }) => void) | null;
  onerror: ((ev: never) => void) | null;
  send(data: string): void;
  close(code?: number, reason?: string): void;
}

export type WebSocketFactory = (url: string) => WebSocketLike;

/** État de la connexion. */
export type ConnectionState =
  /** Pas encore de `connect()`. */
  | "idle"
  /** Première connexion en cours (WebSocket, `hello`, `identify`). */
  | "connecting"
  /** Identifié, prêt à envoyer des requêtes. */
  | "connected"
  /** Connexion perdue, nouvelle tentative programmée ou en cours. */
  | "reconnecting"
  /** Fermé (par `close()`, erreur fatale ou tentatives épuisées). */
  | "closed";

/** Ce que le serveur a annoncé, une fois identifié. */
export interface ConnectionInfo {
  app: AppInfo;
  protocol: number;
  authRequired: boolean;
  capabilities: string[];
  clientId: string;
}

export interface SuiteClientOptions {
  /** URL WebSocket, ex. `ws://127.0.0.1:4460`. */
  url: string;
  /** Jeton partagé de la suite. */
  token?: string;
  /** Nom affiché du client, ex. « Nino plateau 1 ». */
  name?: string;
  /** Identité de l'app cliente, si c'est une app de la suite. */
  app?: AppInfo;
  /** Recevoir les événements (défaut : vrai). */
  events?: boolean;
  /** Délai de réponse d'une requête, en ms (défaut 10 000). */
  requestTimeoutMs?: number;
  /** Délai pour une tentative de connexion complète jusqu'à `identified`, en ms (défaut 10 000). */
  connectTimeoutMs?: number;
  /** Reconnexion automatique (défaut : activée avec les réglages de {@link DEFAULT_BACKOFF}). */
  reconnect?: boolean | BackoffOptions;
  /** Fabrique de WebSocket (défaut : `globalThis.WebSocket`). */
  webSocket?: WebSocketFactory;
  /** Identifiant d'app attendu (ex. `studio`) : la connexion échoue si le serveur en annonce un autre. */
  expectedApp?: string;
  /** Domaine requis par type de requête ; vérifié avant l'envoi si le serveur a annoncé ses domaines. */
  requestCapabilities?: Readonly<Record<string, string>>;
}

export interface RequestOptions {
  /** Délai propre à cette requête, en ms. */
  timeoutMs?: number;
  /** Annulation par l'appelant. */
  signal?: AbortSignal;
}

export type EventHandler = (data: unknown, event: EventMessage) => void;
export type StateListener = (state: ConnectionState, info: ConnectionInfo | null) => void;
export type ErrorListener = (error: SuiteError) => void;

interface Pending {
  type: string;
  resolve: (value: unknown) => void;
  reject: (error: SuiteError) => void;
  cleanup: () => void;
}

interface Waiter {
  resolve: (info: ConnectionInfo) => void;
  reject: (error: SuiteError) => void;
}

interface Subscription {
  app: string;
  type: string;
  handler: EventHandler;
}

const OPEN = 1;

function defaultFactory(): WebSocketFactory {
  const Ctor = (globalThis as { WebSocket?: new (url: string) => WebSocketLike }).WebSocket;
  if (!Ctor) {
    throw new SuiteError(ClientErrorCode.NoWebSocket, "aucun WebSocket disponible dans cet environnement");
  }
  return (url) => new Ctor(url);
}

export class SuiteClient {
  readonly options: Readonly<SuiteClientOptions>;

  private ws: WebSocketLike | null = null;
  private stateValue: ConnectionState = "idle";
  private infoValue: ConnectionInfo | null = null;
  private hello: HelloMessage | null = null;
  private wantOpen = false;
  private fatal: SuiteError | null = null;
  private attempt = 0;
  private nextId = 1;
  private connectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly pending = new Map<string, Pending>();
  private waiters: Waiter[] = [];
  private subscriptions: Subscription[] = [];
  private stateListeners: StateListener[] = [];
  private errorListeners: ErrorListener[] = [];

  constructor(options: SuiteClientOptions) {
    this.options = { ...options };
  }

  /** État courant de la connexion. */
  get state(): ConnectionState {
    return this.stateValue;
  }

  /** Infos du serveur, une fois identifié (conservées pendant une reconnexion). */
  get info(): ConnectionInfo | null {
    return this.infoValue;
  }

  /** Vrai si le client est identifié et peut envoyer des requêtes. */
  get connected(): boolean {
    return this.stateValue === "connected";
  }

  /**
   * Ouvre la connexion et attend `identified`.
   *
   * Avec la reconnexion activée, les échecs passagers sont retentés : la promesse
   * ne se résout qu'à la première identification réussie, ou échoue sur une erreur
   * fatale (`auth_failed`, `protocol_mismatch`…) ou sur `close()`.
   */
  connect(): Promise<ConnectionInfo> {
    if (this.stateValue === "connected" && this.infoValue) {
      return Promise.resolve(this.infoValue);
    }
    const ready = this.waitReady();
    if (!this.wantOpen) {
      this.wantOpen = true;
      this.fatal = null;
      this.attempt = 0;
      this.openSocket();
    }
    return ready;
  }

  /** Ferme la connexion et annule la reconnexion. Les requêtes en cours échouent. */
  close(): void {
    this.wantOpen = false;
    this.clearTimers();
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      this.detach(ws);
      try {
        ws.close(1000, "fermeture demandée");
      } catch {
        // déjà fermé
      }
    }
    const err = new SuiteError(ClientErrorCode.NotConnected, "connexion fermée par le client");
    this.rejectPending(err);
    this.rejectWaiters(err);
    this.setState("closed");
  }

  /** Vrai si le serveur propose ce domaine (ex. `studio.outputs`). Faux tant qu'on n'est pas identifié. */
  hasCapability(capability: string): boolean {
    return this.infoValue?.capabilities.includes(capability) ?? false;
  }

  /** Lève `capability_missing` si le serveur ne propose pas ce domaine. */
  requireCapability(capability: string): void {
    if (!this.hasCapability(capability)) {
      throw new SuiteError(
        ClientErrorCode.CapabilityMissing,
        `le serveur ne propose pas le domaine « ${capability} »`,
      );
    }
  }

  /**
   * Envoie une requête et attend sa réponse.
   * Échoue avec une {@link SuiteError} : code du serveur, ou `timeout`, `not_connected`,
   * `connection_closed`, `capability_missing`, `aborted`.
   */
  async request<T = unknown>(type: string, data?: unknown, options: RequestOptions = {}): Promise<T> {
    const timeoutMs = options.timeoutMs ?? this.options.requestTimeoutMs ?? 10_000;
    const deadline = Date.now() + timeoutMs;
    if (options.signal?.aborted) {
      throw new SuiteError(ClientErrorCode.Aborted, `requête annulée : ${type}`, { requestType: type });
    }
    if (this.stateValue !== "connected") {
      if (!this.wantOpen) {
        throw new SuiteError(ClientErrorCode.NotConnected, `non connecté (requête ${type})`, { requestType: type });
      }
      // Connexion ou reconnexion en cours : on attend, dans la limite du délai de la requête.
      await this.waitReadyWithin(timeoutMs, type, options.signal);
    }
    this.checkCapability(type);
    return this.send<T>(type, data, Math.max(0, deadline - Date.now()), options.signal);
  }

  /**
   * S'abonne aux événements. `app` et `type` acceptent `*`.
   * Renvoie la fonction de désabonnement.
   */
  on(app: string, type: string, handler: EventHandler): () => void {
    const sub: Subscription = { app, type, handler };
    this.subscriptions.push(sub);
    return () => {
      this.subscriptions = this.subscriptions.filter((s) => s !== sub);
    };
  }

  /** S'abonne à tous les événements de toutes les apps. */
  onAnyEvent(handler: EventHandler): () => void {
    return this.on("*", "*", handler);
  }

  /** Attend le prochain événement correspondant (utile dans les scripts et les tests). */
  waitForEvent<D = unknown>(
    app: string,
    type: string,
    options: { timeoutMs?: number; predicate?: (data: D) => boolean } = {},
  ): Promise<D> {
    const timeoutMs = options.timeoutMs ?? this.options.requestTimeoutMs ?? 10_000;
    return new Promise<D>((resolve, reject) => {
      const timer = setTimeout(() => {
        off();
        reject(new SuiteError(ClientErrorCode.Timeout, `événement ${app}.${type} non reçu à temps`));
      }, timeoutMs);
      const off = this.on(app, type, (data) => {
        if (options.predicate && !options.predicate(data as D)) return;
        clearTimeout(timer);
        off();
        resolve(data as D);
      });
    });
  }

  /** Suit les changements d'état de la connexion. */
  onState(listener: StateListener): () => void {
    this.stateListeners.push(listener);
    return () => {
      this.stateListeners = this.stateListeners.filter((l) => l !== listener);
    };
  }

  /** Erreurs hors requête : échec de connexion, erreur fatale, gestionnaire d'événement qui lève. */
  onError(listener: ErrorListener): () => void {
    this.errorListeners.push(listener);
    return () => {
      this.errorListeners = this.errorListeners.filter((l) => l !== listener);
    };
  }

  // --- interne -------------------------------------------------------------

  private get reconnectOptions(): BackoffOptions | null {
    const r = this.options.reconnect ?? true;
    if (r === false) return null;
    return r === true ? DEFAULT_BACKOFF : { ...DEFAULT_BACKOFF, ...r };
  }

  private checkCapability(type: string): void {
    const cap = this.options.requestCapabilities?.[type];
    if (cap && this.infoValue && !this.infoValue.capabilities.includes(cap)) {
      throw new SuiteError(
        ClientErrorCode.CapabilityMissing,
        `le serveur ne propose pas le domaine « ${cap} » nécessaire à ${type}`,
        { requestType: type },
      );
    }
  }

  private send<T>(type: string, data: unknown, timeoutMs: number, signal?: AbortSignal): Promise<T> {
    const ws = this.ws;
    if (!ws || ws.readyState !== OPEN) {
      return Promise.reject(
        new SuiteError(ClientErrorCode.NotConnected, `non connecté (requête ${type})`, { requestType: type }),
      );
    }
    const id = String(this.nextId++);
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        cleanup();
        reject(new SuiteError(ClientErrorCode.Timeout, `pas de réponse à ${type} en ${timeoutMs} ms`, { requestType: type }));
      }, timeoutMs);
      const onAbort = () => {
        this.pending.delete(id);
        cleanup();
        reject(new SuiteError(ClientErrorCode.Aborted, `requête annulée : ${type}`, { requestType: type }));
      };
      const cleanup = () => {
        clearTimeout(timer);
        signal?.removeEventListener("abort", onAbort);
      };
      signal?.addEventListener("abort", onAbort, { once: true });
      this.pending.set(id, { type, resolve: resolve as (v: unknown) => void, reject, cleanup });
      const msg: ClientMessage = { op: "request", id, type };
      if (data !== undefined) msg.data = data;
      try {
        ws.send(JSON.stringify(msg));
      } catch (e) {
        this.pending.delete(id);
        cleanup();
        reject(new SuiteError(ClientErrorCode.ConnectionClosed, `envoi impossible : ${String(e)}`, { requestType: type }));
      }
    });
  }

  private waitReady(): Promise<ConnectionInfo> {
    return new Promise<ConnectionInfo>((resolve, reject) => {
      this.waiters.push({ resolve, reject });
    });
  }

  private waitReadyWithin(timeoutMs: number, type: string, signal?: AbortSignal): Promise<ConnectionInfo> {
    return new Promise<ConnectionInfo>((resolve, reject) => {
      const waiter: Waiter = {
        resolve: (info) => {
          done();
          resolve(info);
        },
        reject: (err) => {
          done();
          reject(err);
        },
      };
      const timer = setTimeout(() => {
        this.waiters = this.waiters.filter((w) => w !== waiter);
        done();
        reject(new SuiteError(ClientErrorCode.Timeout, `connexion non rétablie à temps (requête ${type})`, { requestType: type }));
      }, timeoutMs);
      const onAbort = () => {
        this.waiters = this.waiters.filter((w) => w !== waiter);
        done();
        reject(new SuiteError(ClientErrorCode.Aborted, `requête annulée : ${type}`, { requestType: type }));
      };
      const done = () => {
        clearTimeout(timer);
        signal?.removeEventListener("abort", onAbort);
      };
      signal?.addEventListener("abort", onAbort, { once: true });
      this.waiters.push(waiter);
    });
  }

  private setState(state: ConnectionState): void {
    if (this.stateValue === state) return;
    this.stateValue = state;
    for (const l of [...this.stateListeners]) {
      try {
        l(state, this.infoValue);
      } catch (e) {
        this.reportError(new SuiteError("listener_failed", `écouteur d'état en échec : ${String(e)}`));
      }
    }
  }

  private reportError(error: SuiteError): void {
    for (const l of [...this.errorListeners]) {
      try {
        l(error);
      } catch {
        // un écouteur d'erreurs ne doit pas casser le client
      }
    }
  }

  private clearTimers(): void {
    if (this.connectTimer) clearTimeout(this.connectTimer);
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.connectTimer = null;
    this.reconnectTimer = null;
  }

  private detach(ws: WebSocketLike): void {
    ws.onopen = null;
    ws.onmessage = null;
    ws.onclose = null;
    ws.onerror = null;
  }

  private openSocket(): void {
    this.setState(this.infoValue === null ? "connecting" : "reconnecting");
    this.hello = null;
    let ws: WebSocketLike;
    try {
      ws = (this.options.webSocket ?? defaultFactory())(this.options.url);
    } catch (e) {
      const err =
        e instanceof SuiteError ? e : new SuiteError(ClientErrorCode.ConnectionClosed, `connexion impossible : ${String(e)}`);
      if (err.code === ClientErrorCode.NoWebSocket) this.fatal = err;
      this.reportError(err);
      this.handleClose();
      return;
    }
    this.ws = ws;
    const timeoutMs = this.options.connectTimeoutMs ?? 10_000;
    this.connectTimer = setTimeout(() => {
      this.connectTimer = null;
      this.reportError(new SuiteError(ClientErrorCode.Timeout, `pas d'identification en ${timeoutMs} ms`));
      this.dropSocket(ws);
    }, timeoutMs);
    ws.onmessage = (ev) => this.handleMessage(ws, ev.data);
    ws.onclose = () => {
      if (this.ws === ws) this.dropSocket(ws);
    };
    ws.onerror = () => {
      // L'événement `close` suit toujours : c'est lui qui déclenche la reconnexion.
    };
  }

  /** Abandonne ce WebSocket et décide de la suite (reconnexion ou fermeture). */
  private dropSocket(ws: WebSocketLike): void {
    if (this.ws !== ws) return;
    this.detach(ws);
    try {
      ws.close();
    } catch {
      // déjà fermé
    }
    this.ws = null;
    this.handleClose();
  }

  private handleClose(): void {
    if (this.connectTimer) clearTimeout(this.connectTimer);
    this.connectTimer = null;
    this.rejectPending(new SuiteError(ClientErrorCode.ConnectionClosed, "connexion perdue avant la réponse"));
    if (!this.wantOpen) return;
    const backoff = this.reconnectOptions;
    const maxAttempts = backoff?.maxAttempts ?? DEFAULT_BACKOFF.maxAttempts;
    if (this.fatal || !backoff || this.attempt >= maxAttempts) {
      this.wantOpen = false;
      const err = this.fatal ?? new SuiteError(ClientErrorCode.ConnectionClosed, "connexion fermée");
      this.rejectWaiters(err);
      this.setState("closed");
      return;
    }
    const delay = backoffDelay(this.attempt, backoff);
    this.attempt += 1;
    // Tant qu'on n'a jamais été identifié, on reste « en connexion ».
    this.setState(this.infoValue === null ? "connecting" : "reconnecting");
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (this.wantOpen) this.openSocket();
    }, delay);
  }

  private failFatal(ws: WebSocketLike, error: SuiteError): void {
    this.fatal = error;
    this.reportError(error);
    this.dropSocket(ws);
  }

  private handleMessage(ws: WebSocketLike, raw: unknown): void {
    if (this.ws !== ws) return;
    const msg = parseServerMessage(raw);
    if (!msg) return;
    switch (msg.op) {
      case "hello": {
        if (msg.protocol !== PROTOCOL_VERSION) {
          this.failFatal(
            ws,
            new SuiteError(
              ClientErrorCode.ProtocolMismatch,
              `version de protocole incompatible : serveur ${msg.protocol}, client ${PROTOCOL_VERSION}`,
            ),
          );
          return;
        }
        const expected = this.options.expectedApp;
        if (expected && msg.app?.id !== expected) {
          this.failFatal(
            ws,
            new SuiteError(
              ClientErrorCode.ProtocolMismatch,
              `app inattendue : « ${msg.app?.id} » au lieu de « ${expected} »`,
            ),
          );
          return;
        }
        this.hello = msg;
        const identify: ClientMessage = { op: "identify" };
        if (this.options.token !== undefined) identify.token = this.options.token;
        if (this.options.name !== undefined) identify.name = this.options.name;
        if (this.options.app !== undefined) identify.app = this.options.app;
        if (this.options.events !== undefined) identify.events = this.options.events;
        ws.send(JSON.stringify(identify));
        return;
      }
      case "identified": {
        if (!this.hello) return;
        if (this.connectTimer) clearTimeout(this.connectTimer);
        this.connectTimer = null;
        this.attempt = 0;
        this.infoValue = {
          app: this.hello.app,
          protocol: this.hello.protocol,
          authRequired: this.hello.auth_required,
          capabilities: [...(this.hello.capabilities ?? [])],
          clientId: msg.client_id,
        };
        const info = this.infoValue;
        const waiters = this.waiters;
        this.waiters = [];
        // L'état passe à « connecté » avant de libérer les attentes, pour que
        // les requêtes relancées par les écouteurs partent tout de suite.
        this.setState("connected");
        for (const w of waiters) w.resolve(info);
        return;
      }
      case "response":
        this.handleResponse(ws, msg);
        return;
      case "event":
        this.dispatchEvent(msg);
        return;
    }
  }

  private handleResponse(ws: WebSocketLike, msg: ResponseMessage): void {
    const p = this.pending.get(msg.id);
    if (!p) {
      // Erreur sans requête : réponse à `identify` ou message invalide.
      if (!msg.ok && msg.error) {
        const err = new SuiteError(msg.error.code, msg.error.message, { fromServer: true });
        if (msg.error.code === ProtocolErrorCode.AuthFailed) {
          this.failFatal(ws, err);
        } else {
          this.reportError(err);
        }
      }
      return;
    }
    this.pending.delete(msg.id);
    p.cleanup();
    if (msg.ok) {
      p.resolve(msg.data ?? null);
    } else {
      const code = msg.error?.code ?? "unknown_error";
      const message = msg.error?.message ?? "erreur inconnue";
      p.reject(new SuiteError(code, message, { requestType: p.type, fromServer: true }));
    }
  }

  private dispatchEvent(msg: EventMessage): void {
    for (const s of [...this.subscriptions]) {
      if ((s.app === "*" || s.app === msg.app) && (s.type === "*" || s.type === msg.type)) {
        try {
          s.handler(msg.data, msg);
        } catch (e) {
          this.reportError(
            new SuiteError("listener_failed", `gestionnaire de ${msg.app}.${msg.type} en échec : ${String(e)}`),
          );
        }
      }
    }
  }

  private rejectPending(err: SuiteError): void {
    const all = [...this.pending.values()];
    this.pending.clear();
    for (const p of all) {
      p.cleanup();
      p.reject(new SuiteError(err.code, err.message, { requestType: p.type }));
    }
  }

  private rejectWaiters(err: SuiteError): void {
    const waiters = this.waiters;
    this.waiters = [];
    for (const w of waiters) w.reject(err);
  }
}
