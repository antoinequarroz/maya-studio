/**
 * Test d'intégration : lance le vrai binaire `studiod` sur un port libre avec un
 * jeton, et le pilote avec le client (WebSocket global de Node 22).
 *
 * Binaire utilisé : `STUDIOD_BIN` s'il est défini, sinon
 * `$CARGO_TARGET_DIR/debug/studiod` (ou `target/debug/studiod`), compilé avec
 * `cargo build -p studiod` s'il manque. L'enregistrement demande GStreamer et un encodeur
 * H.264 (choix automatique du moteur).
 */
import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, statSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { isSuiteError, StudioClient, applyStudioEvent, type StudioState } from "../../src/index.js";

const repoRoot = resolve(fileURLToPath(new URL(".", import.meta.url)), "../../../..");
const exe = process.platform === "win32" ? "studiod.exe" : "studiod";

function studiodBinary(): string {
  if (process.env.STUDIOD_BIN) return process.env.STUDIOD_BIN;
  const targetDir = process.env.CARGO_TARGET_DIR ?? join(repoRoot, "target");
  const bin = join(targetDir, "debug", exe);
  if (!existsSync(bin)) {
    execFileSync("cargo", ["build", "-p", "studiod"], { cwd: repoRoot, stdio: "inherit" });
  }
  return bin;
}

function freePort(): Promise<number> {
  return new Promise((ok, ko) => {
    const srv = createServer();
    srv.once("error", ko);
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      srv.close(() => ok(port));
    });
  });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

let child: ChildProcess;
let url: string;
let recordings: string;
let logs = "";
const token = `jeton-${Math.random().toString(36).slice(2)}`;

beforeAll(async () => {
  const bin = studiodBinary();
  const port = await freePort();
  recordings = mkdtempSync(join(tmpdir(), "maya-it-"));
  url = `ws://127.0.0.1:${port}`;
  child = spawn(bin, ["--listen", `127.0.0.1:${port}`, "--token", token, "--recordings", recordings], {
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, RUST_LOG: "info" },
  });
  child.stdout?.on("data", (d: Buffer) => (logs += d.toString()));
  child.stderr?.on("data", (d: Buffer) => (logs += d.toString()));
});

afterAll(() => {
  child?.kill();
  if (recordings) rmSync(recordings, { recursive: true, force: true });
});

function newClient(extra: Partial<ConstructorParameters<typeof StudioClient>[0]> = {}) {
  return new StudioClient({
    url,
    token,
    name: "test d'intégration",
    app: { id: "suite-client-tests", name: "Tests suite-client", version: "0.1.0" },
    // Le serveur démarre en parallèle : on retente vite.
    reconnect: { initialDelayMs: 50, maxDelayMs: 300, jitter: 0, maxAttempts: 100 },
    ...extra,
  });
}

describe("studiod réel", () => {
  it("refuse un mauvais jeton (auth_failed, sans reconnexion)", async () => {
    const bad = newClient({ token: "faux" });
    const err = await bad.connect().catch((e: unknown) => e);
    expect(isSuiteError(err, "auth_failed"), `${String(err)}\n${logs}`).toBe(true);
    expect(bad.state).toBe("closed");
  });

  it("scènes, sources, coupe, capture et enregistrement", async () => {
    const studio = newClient();
    const info = await studio.connect();
    expect(info.app.id).toBe("studio");
    expect(info.protocol).toBe(1);
    expect(info.authRequired).toBe(true);
    expect(studio.hasCapability("studio.outputs")).toBe(true);

    const { requests } = await studio.call("GetRequestList");
    expect(requests).toContain("GetScreenshot");

    // État local tenu uniquement par les événements, à partir d'un instantané.
    let state: StudioState = await studio.call("GetStudio");
    const events: string[] = [];
    studio.subscribe("*", (data, ev) => {
      events.push(ev.type);
      state = applyStudioEvent(state, ev.type, data);
    });

    const { scene } = await studio.call("CreateScene", { name: "Plateau" });
    expect(scene.name).toBe("Plateau");
    const fond = await studio.call("AddSource", { scene: "Plateau", name: "Fond", kind: "color", rgba: [20, 30, 60, 255] });
    expect(fond.source.transform).toEqual({ x: 0, y: 0, width: 1280, height: 720, opacity: 1 });
    await studio.call("AddSource", {
      scene: scene.id,
      name: "Mire",
      kind: "test_pattern",
      transform: { x: 760, y: 40, width: 480, height: 270 },
    });
    const updated = await studio.call("UpdateSource", {
      scene: "Plateau",
      source: "Mire",
      visible: false,
      transform: { x: 700, y: 40, width: 480, height: 270, opacity: 0.5 },
    });
    expect(updated.source.visible).toBe(false);
    expect(updated.source.transform.opacity).toBe(0.5);
    const moved = await studio.call("MoveSource", { scene: "Plateau", source: "Mire", index: 0 });
    expect(moved.source_ids[0]).toBe(updated.source.id);

    await studio.call("SetPreviewScene", { scene: "Plateau" });
    const programChanged = studio.waitForEvent<{ scene_id: string }>("studio", "ProgramSceneChanged");
    await studio.call("Cut");
    expect((await programChanged).scene_id).toBe(scene.id);

    const err = await studio.call("SetProgramScene", { scene: "Plateu" }).catch((e: unknown) => e);
    expect(isSuiteError(err, "scene_not_found")).toBe(true);
    const onAir = await studio.call("RemoveScene", { scene: "Plateau" }).catch((e: unknown) => e);
    expect(isSuiteError(onAir, "scene_on_air")).toBe(true);

    // L'état tenu par les événements correspond à l'état du serveur.
    await sleep(100);
    expect(state).toEqual(await studio.call("GetStudio"));
    expect(state.program_scene).toBe(scene.id);
    expect(events).toEqual(
      expect.arrayContaining(["SceneCreated", "SourceAdded", "SourceUpdated", "SourceReordered", "ProgramSceneChanged", "PreviewSceneChanged"]),
    );

    const jpeg = await studio.call("GetScreenshot", { format: "jpeg", scene: "Plateau" });
    expect(jpeg.image.startsWith("data:image/jpeg;base64,")).toBe(true);
    const png = await studio.call("GetScreenshot");
    const bytes = Buffer.from(png.image.split(",")[1]!, "base64");
    expect(bytes.subarray(1, 4).toString()).toBe("PNG");

    const path = join(recordings, "test.mp4");
    const started = await studio.call("StartRecording", { path });
    expect(started).toMatchObject({ output: "record", target: path, backend: "gstreamer", format: "mp4" });
    expect(started.encoder).toMatch(/\w+/);
    await sleep(1200);
    const status = await studio.call("GetOutputStatus");
    expect(status.record.active).toBe(true);
    expect(status.record.encoder).toBe(started.encoder);
    expect(status.stream.active).toBe(false);
    const again = await studio.call("StartRecording", { path }).catch((e: unknown) => e);
    expect(isSuiteError(again, "output_already_active")).toBe(true);
    const stopped = studio.waitForEvent("studio", "OutputStopped");
    const stop = await studio.call("StopRecording");
    await stopped;
    // En build debug le compositeur logiciel est lent : on vérifie seulement que la sortie a vécu.
    expect(stop.frames, logs).toBeGreaterThan(0);
    expect(stop.duration_s).toBeGreaterThan(1);
    expect(statSync(path).size).toBeGreaterThan(1000);

    studio.close();
  });

  it("encodeurs, périphériques, captures et mixeur", async () => {
    const studio = newClient();
    await studio.connect();
    for (const cap of ["studio.encoders", "studio.inputs", "studio.audio"]) expect(studio.hasCapability(cap)).toBe(true);

    const enc = await studio.call("GetEncoders");
    expect(enc.backend).toBe("gstreamer");
    expect(enc.requested).toBe("auto");
    expect(enc.encoders.filter((e) => e.selected).length).toBeLessThanOrEqual(1);
    const devices = await studio.call("GetInputDevices");
    expect(Array.isArray(devices.video) && Array.isArray(devices.audio)).toBe(true);

    let state: StudioState = await studio.call("GetStudio");
    studio.subscribe("*", (data, ev) => (state = applyStudioEvent(state, ev.type, data)));
    // Fichier absent : la capture échoue et le moteur le signale (puis réessaie).
    const failed = studio.waitForEvent<{ source_id: string; kind: string; error: string; retry_in_ms: number }>(
      "studio",
      "SourceFailed",
    );
    const { source } = await studio.call("AddSource", {
      scene: "Scène 1",
      name: "Clip absent",
      kind: "media_file",
      path: join(recordings, "absent.mp4"),
      loop: true,
    });
    expect(source).toMatchObject({ kind: "media_file", loop: true, volume: 1, muted: false });
    const failure = await failed;
    expect(failure).toMatchObject({ source_id: source.id, kind: "media_file" });
    expect(failure.retry_in_ms).toBeGreaterThanOrEqual(1000);
    expect(failure.error.length).toBeGreaterThan(0);

    const vol = studio.waitForEvent("studio", "SourceVolumeChanged");
    expect(await studio.call("SetSourceVolume", { scene: "Scène 1", source: source.id, volume: 1.5 })).toMatchObject({
      source_id: source.id,
      volume: 1.5,
    });
    await vol;
    await studio.call("SetSourceMuted", { scene: "Scène 1", source: "Clip absent", muted: true });
    const tooLoud = await studio
      .call("SetSourceVolume", { scene: "Scène 1", source: source.id, volume: 3 })
      .catch((e: unknown) => e);
    expect(isSuiteError(tooLoud, "invalid_volume")).toBe(true);
    await sleep(100);
    expect(state).toEqual(await studio.call("GetStudio"));
    await studio.call("RemoveSource", { scene: "Scène 1", source: source.id });

    const mkv = await studio.call("StartRecording", { format: "mkv", bitrate_kbps: 2000 });
    expect(mkv).toMatchObject({ format: "mkv", bitrate_kbps: 2000 });
    expect(mkv.target.endsWith(".mkv")).toBe(true);
    await sleep(500);
    const stop = await studio.call("StopRecording");
    expect(stop.finalized).toBe(true);
    studio.close();
  });

  it("deux clients : les événements de l'un arrivent chez l'autre", async () => {
    const a = newClient();
    const b = newClient({ name: "observateur" });
    await Promise.all([a.connect(), b.connect()]);
    const seen = b.waitForEvent<{ scene: { name: string } }>("studio", "SceneCreated");
    await a.call("CreateScene", { name: "Interview" });
    expect((await seen).scene.name).toBe("Interview");
    await a.call("RenameScene", { scene: "Interview", name: "Entretien" });
    await a.call("RemoveScene", { scene: "Entretien" });
    a.close();
    b.close();
  });
});
