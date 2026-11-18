import type { WebSocketLike } from "../../src/client.js";

/** Faux WebSocket piloté par le test : il joue le rôle du serveur. */
export class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: ((ev: never) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: ((ev: { code?: number; reason?: string }) => void) | null = null;
  onerror: ((ev: never) => void) | null = null;
  readonly sent: Record<string, unknown>[] = [];
  closed = false;

  constructor(readonly url: string) {}

  send(data: string): void {
    if (this.readyState !== 1) throw new Error("socket fermé");
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }

  close(): void {
    this.closed = true;
    this.readyState = 3;
  }

  /** Le serveur accepte la connexion et envoie `hello`. */
  open(hello: Partial<Record<string, unknown>> = {}): void {
    this.readyState = 1;
    this.onopen?.(undefined as never);
    this.serverSend({
      op: "hello",
      app: { id: "studio", name: "Maya Studio", version: "0.1.0" },
      protocol: 1,
      auth_required: true,
      capabilities: ["core", "studio.scenes", "studio.sources", "studio.outputs", "studio.preview"],
      ...hello,
    });
  }

  serverSend(msg: unknown): void {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }

  /** Coupure côté serveur ou réseau. */
  serverClose(): void {
    this.readyState = 3;
    this.onclose?.({ code: 1006 });
  }

  lastSent(): Record<string, unknown> {
    const m = this.sent[this.sent.length - 1];
    if (!m) throw new Error("rien n'a été envoyé");
    return m;
  }

  /** Répond à la dernière requête envoyée. */
  reply(data: unknown, id = this.lastSent().id as string): void {
    this.serverSend({ op: "response", id, ok: true, data });
  }

  replyError(code: string, message: string, id = this.lastSent().id as string): void {
    this.serverSend({ op: "response", id, ok: false, error: { code, message } });
  }
}

export function fakeFactory() {
  const sockets: FakeSocket[] = [];
  const factory = (url: string) => {
    const s = new FakeSocket(url);
    sockets.push(s);
    return s;
  };
  return { sockets, factory };
}

/** Laisse passer les microtâches (résolution des promesses). */
export async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}
