import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SuiteClient, type ConnectionState } from "../../src/client.js";
import { ClientErrorCode, SuiteError, isSuiteError } from "../../src/errors.js";
import { StudioClient } from "../../src/studio.js";
import { fakeFactory, flush, type FakeSocket } from "./fake-socket.js";

function setup(extra: Partial<ConstructorParameters<typeof SuiteClient>[0]> = {}) {
  const { sockets, factory } = fakeFactory();
  const client = new SuiteClient({
    url: "ws://test",
    token: "secret",
    name: "test",
    app: { id: "nino", name: "Nino", version: "0.3.0" },
    webSocket: factory,
    reconnect: { initialDelayMs: 100, maxDelayMs: 1000, jitter: 0 },
    ...extra,
  });
  return { client, sockets };
}

async function connected(extra: Partial<ConstructorParameters<typeof SuiteClient>[0]> = {}) {
  const ctx = setup(extra);
  const p = ctx.client.connect();
  const ws = ctx.sockets[0]!;
  ws.open();
  ws.serverSend({ op: "identified", client_id: "7" });
  await p;
  return { ...ctx, ws };
}

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe("poignée de main", () => {
  it("envoie identify avec jeton, nom et app, puis résout les infos", async () => {
    const { client, sockets } = setup();
    const states: ConnectionState[] = [];
    client.onState((s) => states.push(s));
    const p = client.connect();
    expect(client.state).toBe("connecting");
    const ws = sockets[0]!;
    ws.open();
    expect(ws.lastSent()).toEqual({
      op: "identify",
      token: "secret",
      name: "test",
      app: { id: "nino", name: "Nino", version: "0.3.0" },
    });
    ws.serverSend({ op: "identified", client_id: "3" });
    const info = await p;
    expect(info.clientId).toBe("3");
    expect(info.app.id).toBe("studio");
    expect(info.authRequired).toBe(true);
    expect(client.connected).toBe(true);
    expect(client.hasCapability("studio.outputs")).toBe(true);
    expect(client.hasCapability("nino.rundown")).toBe(false);
    expect(() => client.requireCapability("nino.rundown")).toThrowError(SuiteError);
    expect(states).toEqual(["connecting", "connected"]);
  });

  it("refuse une version majeure différente sans reconnecter", async () => {
    const { client, sockets } = setup();
    const p = client.connect();
    sockets[0]!.open({ protocol: 2 });
    await expect(p).rejects.toMatchObject({ code: ClientErrorCode.ProtocolMismatch });
    expect(client.state).toBe("closed");
    await vi.advanceTimersByTimeAsync(5000);
    expect(sockets).toHaveLength(1);
  });

  it("refuse une app inattendue", async () => {
    const { client, sockets } = setup({ expectedApp: "nino" });
    const p = client.connect();
    sockets[0]!.open();
    await expect(p).rejects.toMatchObject({ code: ClientErrorCode.ProtocolMismatch });
  });

  it("auth_failed est fatal : pas de reconnexion", async () => {
    const { client, sockets } = setup();
    const errors: string[] = [];
    client.onError((e) => errors.push(e.code));
    const p = client.connect();
    const ws = sockets[0]!;
    ws.open();
    ws.serverSend({ op: "response", id: "", ok: false, error: { code: "auth_failed", message: "jeton invalide" } });
    await expect(p).rejects.toMatchObject({ code: "auth_failed", fromServer: true });
    expect(errors).toContain("auth_failed");
    expect(client.state).toBe("closed");
    await vi.advanceTimersByTimeAsync(5000);
    expect(sockets).toHaveLength(1);
  });

  it("échoue si identified n'arrive pas à temps (sans reconnexion)", async () => {
    const { client, sockets } = setup({ reconnect: false, connectTimeoutMs: 500 });
    const p = client.connect();
    sockets[0]!.open();
    const assertion = expect(p).rejects.toMatchObject({ code: ClientErrorCode.ConnectionClosed });
    await vi.advanceTimersByTimeAsync(600);
    await assertion;
    expect(sockets[0]!.closed).toBe(true);
  });

  it("n'envoie pas events par défaut, mais le transmet s'il est donné", async () => {
    const { client, sockets } = setup({ events: false });
    void client.connect();
    sockets[0]!.open();
    expect(sockets[0]!.lastSent().events).toBe(false);
  });
});

describe("requêtes", () => {
  it("numérote les requêtes et résout les réponses dans le désordre", async () => {
    const { client, ws } = await connected();
    const a = client.request<{ n: number }>("GetScene", { scene: "A" });
    const b = client.request<{ n: number }>("GetScene", { scene: "B" });
    await flush();
    const [ma, mb] = ws.sent.slice(-2);
    expect(ma).toMatchObject({ op: "request", type: "GetScene", data: { scene: "A" } });
    expect(ma!.id).not.toBe(mb!.id);
    ws.reply({ n: 2 }, mb!.id as string);
    ws.reply({ n: 1 }, ma!.id as string);
    expect(await a).toEqual({ n: 1 });
    expect(await b).toEqual({ n: 2 });
  });

  it("omet data quand il n'y en a pas", async () => {
    const { client, ws } = await connected();
    const p = client.request("Ping");
    await flush();
    expect("data" in ws.lastSent()).toBe(false);
    ws.reply({ pong: true });
    await expect(p).resolves.toEqual({ pong: true });
  });

  it("transforme une réponse ok:false en SuiteError typée", async () => {
    const { client, ws } = await connected();
    const p = client.request("SetProgramScene", { scene: "Plateu" });
    await flush();
    ws.replyError("scene_not_found", "scène introuvable : Plateu");
    const err = await p.catch((e: unknown) => e);
    expect(isSuiteError(err, "scene_not_found")).toBe(true);
    expect(err).toMatchObject({ requestType: "SetProgramScene", fromServer: true, message: "scène introuvable : Plateu" });
  });

  it("expire après le délai", async () => {
    const { client } = await connected({ requestTimeoutMs: 1000 });
    const p = client.request("Ping");
    const assertion = expect(p).rejects.toMatchObject({ code: ClientErrorCode.Timeout, requestType: "Ping" });
    await vi.advanceTimersByTimeAsync(1001);
    await assertion;
  });

  it("accepte un délai par requête", async () => {
    const { client } = await connected({ requestTimeoutMs: 60_000 });
    const p = client.request("Ping", undefined, { timeoutMs: 50 });
    const assertion = expect(p).rejects.toMatchObject({ code: ClientErrorCode.Timeout });
    await vi.advanceTimersByTimeAsync(60);
    await assertion;
  });

  it("ignore une réponse tardive après expiration", async () => {
    const { client, ws } = await connected({ requestTimeoutMs: 10 });
    const p = client.request("Ping").catch((e: unknown) => e);
    await flush();
    const id = ws.lastSent().id as string;
    await vi.advanceTimersByTimeAsync(20);
    expect(await p).toBeInstanceOf(SuiteError);
    expect(() => ws.reply({ pong: true }, id)).not.toThrow();
  });

  it("peut être annulée", async () => {
    const { client } = await connected();
    const ctrl = new AbortController();
    const p = client.request("Ping", undefined, { signal: ctrl.signal });
    await flush();
    ctrl.abort();
    await expect(p).rejects.toMatchObject({ code: ClientErrorCode.Aborted });
  });

  it("refuse si jamais connecté", async () => {
    const { client } = setup();
    await expect(client.request("Ping")).rejects.toMatchObject({ code: ClientErrorCode.NotConnected });
  });

  it("close() fait échouer les requêtes en cours et les suivantes", async () => {
    const { client, ws } = await connected();
    const p = client.request("Ping");
    await flush();
    client.close();
    await expect(p).rejects.toMatchObject({ code: ClientErrorCode.NotConnected });
    await expect(client.request("Ping")).rejects.toMatchObject({ code: ClientErrorCode.NotConnected });
    expect(client.state).toBe("closed");
    expect(ws.closed).toBe(true);
  });

  it("vérifie le domaine requis avant l'envoi", async () => {
    const { sockets, factory } = fakeFactory();
    const studio = new StudioClient({ url: "ws://x", webSocket: factory });
    const p = studio.connect();
    sockets[0]!.open({ capabilities: ["core", "studio.scenes"] });
    sockets[0]!.serverSend({ op: "identified", client_id: "1" });
    await p;
    await expect(studio.call("StartRecording")).rejects.toMatchObject({
      code: ClientErrorCode.CapabilityMissing,
      requestType: "StartRecording",
    });
    const sentBefore = sockets[0]!.sent.length;
    const cut = studio.call("Cut");
    await flush();
    expect(sockets[0]!.sent.length).toBe(sentBefore + 1);
    sockets[0]!.reply({});
    await expect(cut).resolves.toEqual({});
  });
});

describe("événements", () => {
  it("filtre par app et par type, avec jokers et désabonnement", async () => {
    const { client, ws } = await connected();
    const seen: string[] = [];
    const off = client.on("studio", "SceneCreated", (d) => seen.push(`exact:${JSON.stringify(d)}`));
    client.on("studio", "*", (_d, e) => seen.push(`studio:${e.type}`));
    client.on("*", "Tally", (_d, e) => seen.push(`tally:${e.app}`));
    client.onAnyEvent((_d, e) => seen.push(`any:${e.app}.${e.type}`));
    ws.serverSend({ op: "event", app: "studio", type: "SceneCreated", data: { x: 1 } });
    ws.serverSend({ op: "event", app: "nino", type: "Tally", data: {} });
    off();
    ws.serverSend({ op: "event", app: "studio", type: "SceneCreated", data: { x: 2 } });
    expect(seen).toEqual([
      'exact:{"x":1}',
      "studio:SceneCreated",
      "any:studio.SceneCreated",
      "tally:nino",
      "any:nino.Tally",
      "studio:SceneCreated",
      "any:studio.SceneCreated",
    ]);
  });

  it("un gestionnaire qui lève n'empêche pas les autres", async () => {
    const { client, ws } = await connected();
    const errors: string[] = [];
    client.onError((e) => errors.push(e.code));
    const got: number[] = [];
    client.on("studio", "*", () => {
      throw new Error("boum");
    });
    client.on("studio", "*", () => got.push(1));
    ws.serverSend({ op: "event", app: "studio", type: "Cut", data: {} });
    expect(got).toEqual([1]);
    expect(errors).toEqual(["listener_failed"]);
  });

  it("waitForEvent attend un événement correspondant", async () => {
    const { client, ws } = await connected();
    const p = client.waitForEvent<{ scene_id: string }>("studio", "ProgramSceneChanged", {
      predicate: (d) => d.scene_id === "b",
    });
    ws.serverSend({ op: "event", app: "studio", type: "ProgramSceneChanged", data: { scene_id: "a" } });
    ws.serverSend({ op: "event", app: "studio", type: "ProgramSceneChanged", data: { scene_id: "b" } });
    await expect(p).resolves.toEqual({ scene_id: "b" });
  });

  it("ignore les messages inconnus ou invalides", async () => {
    const { client, ws } = await connected();
    ws.onmessage?.({ data: "pas du json" });
    ws.serverSend({ op: "futur", x: 1 });
    expect(client.connected).toBe(true);
  });
});

describe("reconnexion", () => {
  it("reconnecte avec backoff, ré-identifie et rejette les requêtes en vol", async () => {
    const { client, sockets, ws } = await connected();
    const states: ConnectionState[] = [];
    client.onState((s) => states.push(s));
    const inflight = client.request("Ping");
    await flush();
    ws.serverClose();
    await expect(inflight).rejects.toMatchObject({ code: ClientErrorCode.ConnectionClosed });
    expect(client.state).toBe("reconnecting");
    // Une requête pendant la coupure attend le retour de la connexion.
    const waiting = client.request<{ pong: boolean }>("Ping");

    await vi.advanceTimersByTimeAsync(99);
    expect(sockets).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(sockets).toHaveLength(2);
    // Deuxième échec : délai doublé.
    sockets[1]!.serverClose();
    await vi.advanceTimersByTimeAsync(199);
    expect(sockets).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(sockets).toHaveLength(3);

    const ws3 = sockets[2]!;
    ws3.open();
    expect(ws3.lastSent()).toMatchObject({ op: "identify", token: "secret" });
    ws3.serverSend({ op: "identified", client_id: "8" });
    await flush();
    expect(client.info?.clientId).toBe("8");
    expect(ws3.lastSent()).toMatchObject({ op: "request", type: "Ping" });
    ws3.reply({ pong: true });
    await expect(waiting).resolves.toEqual({ pong: true });
    expect(states).toEqual(["reconnecting", "connected"]);

    // Après succès, le compteur repart de zéro.
    ws3.serverClose();
    await vi.advanceTimersByTimeAsync(100);
    expect(sockets).toHaveLength(4);
    client.close();
  });

  it("connect() retente tant que le serveur n'est pas encore là", async () => {
    const { client, sockets } = setup();
    const p = client.connect();
    sockets[0]!.serverClose();
    expect(client.state).toBe("connecting");
    await vi.advanceTimersByTimeAsync(100);
    sockets[1]!.open();
    sockets[1]!.serverSend({ op: "identified", client_id: "1" });
    await expect(p).resolves.toMatchObject({ clientId: "1" });
  });

  it("s'arrête après maxAttempts", async () => {
    const { client, sockets } = setup({ reconnect: { initialDelayMs: 10, maxAttempts: 2, jitter: 0 } });
    const p = client.connect();
    const assertion = expect(p).rejects.toMatchObject({ code: ClientErrorCode.ConnectionClosed });
    sockets[0]!.serverClose();
    await vi.advanceTimersByTimeAsync(10);
    sockets[1]!.serverClose();
    await vi.advanceTimersByTimeAsync(20);
    sockets[2]!.serverClose();
    await assertion;
    expect(client.state).toBe("closed");
    expect(sockets).toHaveLength(3);
  });

  it("sans reconnexion, une coupure ferme le client", async () => {
    const { client, ws } = await connected({ reconnect: false });
    ws.serverClose();
    expect(client.state).toBe("closed");
    await expect(client.request("Ping")).rejects.toMatchObject({ code: ClientErrorCode.NotConnected });
  });

  it("une requête attend au plus son délai pendant la reconnexion", async () => {
    const { client, ws } = await connected({ requestTimeoutMs: 300 });
    ws.serverClose();
    const p = client.request("Ping");
    const assertion = expect(p).rejects.toMatchObject({ code: ClientErrorCode.Timeout });
    await vi.advanceTimersByTimeAsync(301);
    await assertion;
    client.close();
  });
});

export type { FakeSocket };
