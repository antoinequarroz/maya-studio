import { describe, expect, it } from "vitest";
import {
  applyStudioEvent,
  AUDIO_SOURCE_KINDS,
  CAPTURE_SOURCE_KINDS,
  findScene,
  hasAudio,
  SOURCE_KINDS,
  STUDIO_CAPABILITIES,
  STUDIO_EVENTS,
  STUDIO_REQUESTS,
  STUDIO_REQUEST_CAPABILITIES,
  type Source,
  type StudioState,
} from "../../src/studio.js";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const full = { x: 0, y: 0, width: 1280, height: 720, opacity: 1 };

function src(id: string, name = id): Source {
  return { id, name, kind: "test_pattern", transform: full, visible: true };
}

function initial(): StudioState {
  return {
    canvas: { width: 1280, height: 720, fps: 30 },
    scenes: [
      { id: "s1", name: "Scène 1", sources: [src("a"), src("b")] },
      { id: "s2", name: "Plateau", sources: [] },
    ],
    program_scene: "s1",
    preview_scene: "s2",
  };
}

describe("applyStudioEvent", () => {
  it("scènes : création, renommage, suppression", () => {
    let s = initial();
    s = applyStudioEvent(s, "SceneCreated", { scene: { id: "s3", name: "Interview", sources: [] } });
    expect(s.scenes.map((x) => x.name)).toEqual(["Scène 1", "Plateau", "Interview"]);
    // Idempotent : un doublon ne crée rien.
    expect(applyStudioEvent(s, "SceneCreated", { scene: { id: "s3", name: "Interview", sources: [] } })).toBe(s);
    s = applyStudioEvent(s, "SceneRenamed", { scene_id: "s3", name: "Entretien" });
    expect(findScene(s, "s3")?.name).toBe("Entretien");
    s = applyStudioEvent(s, "SceneRemoved", { scene_id: "s2" });
    expect(s.scenes.map((x) => x.id)).toEqual(["s1", "s3"]);
    // L'aperçu supprimé retombe sur le programme, comme dans le moteur.
    expect(s.preview_scene).toBe("s1");
  });

  it("programme, aperçu et canvas", () => {
    let s = initial();
    s = applyStudioEvent(s, "ProgramSceneChanged", { scene_id: "s2" });
    s = applyStudioEvent(s, "PreviewSceneChanged", { scene_id: "s1" });
    expect([s.program_scene, s.preview_scene]).toEqual(["s2", "s1"]);
    expect(applyStudioEvent(s, "PreviewSceneChanged", { scene_id: "s1" })).toBe(s);
    s = applyStudioEvent(s, "CanvasChanged", { width: 1920, height: 1080, fps: 60 });
    expect(s.canvas).toEqual({ width: 1920, height: 1080, fps: 60 });
  });

  it("sources : ajout, mise à jour, ordre, suppression, sans muter l'état d'origine", () => {
    const before = initial();
    let s = applyStudioEvent(before, "SourceAdded", { scene_id: "s1", source: src("c") });
    expect(before.scenes[0]!.sources).toHaveLength(2);
    expect(s.scenes[0]!.sources.map((x) => x.id)).toEqual(["a", "b", "c"]);
    s = applyStudioEvent(s, "SourceUpdated", { scene_id: "s1", source: { ...src("b"), visible: false } });
    expect(s.scenes[0]!.sources[1]!.visible).toBe(false);
    s = applyStudioEvent(s, "SourceReordered", { scene_id: "s1", source_ids: ["c", "a", "b"] });
    expect(s.scenes[0]!.sources.map((x) => x.id)).toEqual(["c", "a", "b"]);
    s = applyStudioEvent(s, "SourceRemoved", { scene_id: "s1", source_id: "a" });
    expect(s.scenes[0]!.sources.map((x) => x.id)).toEqual(["c", "b"]);
    expect(s.scenes[1]).toBe(before.scenes[1]);
  });

  it("volume et mute d'une source", () => {
    const before = initial();
    let s = applyStudioEvent(before, "SourceVolumeChanged", { scene_id: "s1", source_id: "b", volume: 0.5 });
    expect(s.scenes[0]!.sources[1]!.volume).toBe(0.5);
    expect(before.scenes[0]!.sources[1]!.volume).toBeUndefined();
    expect(s.scenes[0]!.sources[0]).toBe(before.scenes[0]!.sources[0]);
    s = applyStudioEvent(s, "SourceMuteChanged", { scene_id: "s1", source_id: "b", muted: true });
    expect(s.scenes[0]!.sources[1]!.muted).toBe(true);
    // Sans changement réel, ou source inconnue : même état.
    expect(applyStudioEvent(s, "SourceMuteChanged", { scene_id: "s1", source_id: "b", muted: true })).toBe(s);
    expect(applyStudioEvent(s, "SourceVolumeChanged", { scene_id: "s1", source_id: "zz", volume: 1 })).toBe(s);
    // Les niveaux et l'état des captures ne touchent pas l'état du studio.
    expect(applyStudioEvent(s, "InputLevels", { sources: [] })).toBe(s);
    expect(applyStudioEvent(s, "SourceStarted", { source_id: "b", kind: "camera" })).toBe(s);
  });

  it("entrées audio globales : création, réglages, suppression", () => {
    let s = initial();
    expect(s.audio_inputs).toBeUndefined();
    const mic = { id: "m1", name: "Micro Anne", kind: "audio_input" as const, volume: 1, muted: false };
    s = applyStudioEvent(s, "AudioInputCreated", { input: mic });
    expect(s.audio_inputs).toEqual([mic]);
    expect(applyStudioEvent(s, "AudioInputCreated", { input: mic })).toBe(s);
    s = applyStudioEvent(s, "AudioInputMuteChanged", { input_id: "m1", muted: true });
    expect(s.audio_inputs![0]!.muted).toBe(true);
    expect(applyStudioEvent(s, "AudioInputMuteChanged", { input_id: "m1", muted: true })).toBe(s);
    s = applyStudioEvent(s, "AudioInputVolumeChanged", { input_id: "m1", volume: 0.5 });
    expect(s.audio_inputs![0]!.volume).toBe(0.5);
    s = applyStudioEvent(s, "AudioInputUpdated", { input: { ...mic, name: "Micro plateau", device_id: "hw:2" } });
    expect(s.audio_inputs![0]!.name).toBe("Micro plateau");
    expect(applyStudioEvent(s, "AudioInputVolumeChanged", { input_id: "zz", volume: 1 })).toBe(s);
    s = applyStudioEvent(s, "AudioInputRemoved", { input_id: "m1" });
    expect(s.audio_inputs).toEqual([]);
    expect(applyStudioEvent(s, "RecordingPaused", { output: "record", paused: true, duration_s: 1 })).toBe(s);
  });

  it("ignore les événements inconnus et les scènes absentes", () => {
    const s = initial();
    expect(applyStudioEvent(s, "OutputStarted", { output: "record", target: "x" })).toBe(s);
    expect(applyStudioEvent(s, "EvenementDuFutur", {})).toBe(s);
    expect(applyStudioEvent(s, "SourceAdded", { scene_id: "zz", source: src("c") })).toBe(s);
  });
});

describe("cohérence avec api.rs", () => {
  const apiRs = readFileSync(
    fileURLToPath(new URL("../../../../crates/studiod/src/api.rs", import.meta.url)),
    "utf8",
  );

  it("toutes les requêtes de REQUESTS sont typées, et réciproquement", () => {
    const block = /pub const REQUESTS: &\[&str\] = &\[([\s\S]*?)\];/.exec(apiRs)?.[1] ?? "";
    const rust = [...block.matchAll(/"(\w+)"/g)].map((m) => m[1]);
    expect(rust.length).toBeGreaterThan(10);
    expect([...STUDIO_REQUESTS].sort()).toEqual([...rust].sort());
  });

  it("les domaines utilisés existent dans CAPABILITIES, et réciproquement", () => {
    const block = /pub const CAPABILITIES: &\[&str\] = &\[([\s\S]*?)\];/.exec(apiRs)?.[1] ?? "";
    const rust = [...block.matchAll(/"([\w.]+)"/g)].map((m) => m[1]!);
    expect([...STUDIO_CAPABILITIES].sort()).toEqual([...rust].sort());
    const caps = new Set(rust);
    for (const cap of Object.values(STUDIO_REQUEST_CAPABILITIES)) expect(caps.has(cap)).toBe(true);
    // Chaque domaine annoncé a au moins une requête typée.
    const used = new Set(Object.values(STUDIO_REQUEST_CAPABILITIES));
    for (const cap of rust) expect(used.has(cap)).toBe(true);
  });

  it("les types de source suivent SourceKind (studio-core)", () => {
    const modelRs = readFileSync(
      fileURLToPath(new URL("../../../../crates/studio-core/src/model.rs", import.meta.url)),
      "utf8",
    );
    const block = /pub enum SourceKind \{([\s\S]*?)\n\}/.exec(modelRs)?.[1] ?? "";
    const snake = (n: string) => n.replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase();
    const variants = [...block.matchAll(/^ {4}(\w+)(?: \{|,)/gm)].map((m) => snake(m[1]!));
    expect(variants.length).toBeGreaterThan(5);
    expect([...SOURCE_KINDS]).toEqual(variants);
    // Mêmes listes que has_audio / is_live_video (is_capture = l'une ou l'autre).
    const fnBlock = (name: string) =>
      [...(new RegExp(`fn ${name}\\(&self\\) -> bool \\{([\\s\\S]*?)\\n {4}\\}`).exec(modelRs)?.[1] ?? "").matchAll(
        /SourceKind::(\w+)/g,
      )].map((m) => snake(m[1]!));
    const audio = fnBlock("has_audio");
    const live = fnBlock("is_live_video");
    expect(audio.length).toBeGreaterThan(0);
    expect([...AUDIO_SOURCE_KINDS].sort()).toEqual([...audio].sort());
    expect([...CAPTURE_SOURCE_KINDS].sort()).toEqual([...new Set([...audio, ...live])].sort());
    expect(hasAudio("media_file")).toBe(true);
    expect(hasAudio("camera")).toBe(false);
  });

  it("les événements émis par le moteur sont typés", () => {
    const engineRs = readFileSync(
      fileURLToPath(new URL("../../../../crates/studiod/src/engine.rs", import.meta.url)),
      "utf8",
    );
    const studioRs = readFileSync(
      fileURLToPath(new URL("../../../../crates/studio-core/src/studio.rs", import.meta.url)),
      "utf8",
    );
    const capturesRs = readFileSync(
      fileURLToPath(new URL("../../../../crates/studiod/src/captures.rs", import.meta.url)),
      "utf8",
    );
    // `engine.emit("X", …)` et `(self.emit)(\n "X", …)` dans les captures.
    const emitted = [...(engineRs + apiRs + capturesRs).matchAll(/emit\)?\(\s*"(\w+)"/g)].map((m) => m[1]!);
    for (const e of ["SourceStarted", "SourceFailed", "InputLevels", "OutputFailed"]) expect(emitted).toContain(e);
    const changeBlock = /pub enum Change \{([\s\S]*?)\n\}/.exec(studioRs)?.[1] ?? "";
    const changes = [...changeBlock.matchAll(/^ {4}(\w+) \{/gm)].map((m) => m[1]!);
    expect(changes.length).toBeGreaterThan(5);
    for (const e of [...emitted, ...changes]) expect(STUDIO_EVENTS).toContain(e);
  });
});
