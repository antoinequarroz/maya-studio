import { SuiteClient, type WebSocketLike } from "@suite-media/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { connectWatchingFatal } from "../src/fatal";

/** Faux serveur minimal : poignée de main et coupures. */
class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: ((ev: never) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: ((ev: { code?: number; reason?: string }) => void) | null = null;
  onerror: ((ev: never) => void) | null = null;
  send(): void {}
  close(): void {
    this.readyState = 3;
  }
  hello(protocol = 1): void {
    this.readyState = 1;
    this.onopen?.(undefined as never);
    this.serverSend({
      op: "hello",
      app: { id: "studio", name: "Maya Studio", version: "0.1.0" },
      protocol,
      auth_required: true,
      capabilities: ["core"],
    });
  }
  serverSend(msg: unknown): void {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }
  serverClose(): void {
    this.readyState = 3;
    this.onclose?.({ code: 1006 });
  }
}

function setup() {
  const sockets: FakeSocket[] = [];
  const client = new SuiteClient({
    url: "ws://test",
    token: "t",
    name: "test",
    app: { id: "studio-ui", name: "UI", version: "0.1.0" },
    webSocket: () => {
      const s = new FakeSocket();
      sockets.push(s);
      return s;
    },
    reconnect: { initialDelayMs: 100, maxDelayMs: 100, jitter: 0 },
  });
  const onFatal = vi.fn();
  const stop = connectWatchingFatal(client, onFatal);
  return { client, sockets, onFatal, stop };
}

async function flush() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe("connectWatchingFatal", () => {
  it("signale un jeton refusé à la première connexion", async () => {
    const { sockets, onFatal } = setup();
    sockets[0]!.hello();
    sockets[0]!.serverSend({ op: "response", id: "", ok: false, error: { code: "auth_failed", message: "jeton invalide" } });
    await flush();
    expect(onFatal).toHaveBeenCalledWith("Jeton refusé par le studio.");
  });

  it("signale un jeton refusé à une reconnexion, après la première connexion", async () => {
    const { sockets, onFatal, client } = setup();
    sockets[0]!.hello();
    sockets[0]!.serverSend({ op: "identified", client_id: "1" });
    await flush();
    expect(client.state).toBe("connected");

    // Le studio redémarre avec un autre jeton.
    sockets[0]!.serverClose();
    await vi.advanceTimersByTimeAsync(150);
    sockets[1]!.hello();
    sockets[1]!.serverSend({ op: "response", id: "", ok: false, error: { code: "auth_failed", message: "jeton invalide" } });
    await flush();
    expect(client.state).toBe("closed");
    expect(onFatal).toHaveBeenCalledTimes(1);
    expect(onFatal).toHaveBeenCalledWith("Jeton refusé par le studio.");
  });

  it("signale une version de protocole incompatible à une reconnexion", async () => {
    const { sockets, onFatal } = setup();
    sockets[0]!.hello();
    sockets[0]!.serverSend({ op: "identified", client_id: "1" });
    await flush();
    sockets[0]!.serverClose();
    await vi.advanceTimersByTimeAsync(150);
    sockets[1]!.hello(99);
    await flush();
    expect(onFatal).toHaveBeenCalledTimes(1);
    expect(onFatal.mock.calls[0]![0]).toMatch(/^Studio incompatible/);
  });

  it("le nettoyage ferme le client sans erreur fatale", async () => {
    const { sockets, onFatal, stop, client } = setup();
    sockets[0]!.hello();
    sockets[0]!.serverSend({ op: "identified", client_id: "1" });
    await flush();
    stop();
    await flush();
    expect(client.state).toBe("closed");
    expect(onFatal).not.toHaveBeenCalled();
  });
});
