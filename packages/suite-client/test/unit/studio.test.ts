import { describe, expect, it } from "vitest";
import {
  applyStudioEvent,
  findScene,
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

  it("les domaines utilisés existent dans CAPABILITIES", () => {
    const block = /pub const CAPABILITIES: &\[&str\] = &\[([\s\S]*?)\];/.exec(apiRs)?.[1] ?? "";
    const caps = new Set([...block.matchAll(/"([\w.]+)"/g)].map((m) => m[1]));
    for (const cap of Object.values(STUDIO_REQUEST_CAPABILITIES)) expect(caps.has(cap)).toBe(true);
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
    const emitted = [...(engineRs + apiRs).matchAll(/emit\("(\w+)"/g)].map((m) => m[1]!);
    const changeBlock = /pub enum Change \{([\s\S]*?)\n\}/.exec(studioRs)?.[1] ?? "";
    const changes = [...changeBlock.matchAll(/^ {4}(\w+) \{/gm)].map((m) => m[1]!);
    expect(changes.length).toBeGreaterThan(5);
    for (const e of [...emitted, ...changes]) expect(STUDIO_EVENTS).toContain(e);
  });
});
