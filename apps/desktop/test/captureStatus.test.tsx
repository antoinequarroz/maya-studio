// @vitest-environment jsdom
import type { Source, StudioClient, StudioState } from "@suite-media/client";
import { act } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { captureSettingsChange, captureSettingsKey } from "../src/studio/captureSettings";
import { StudioProvider, type StudioContextValue, useStudio } from "../src/studio/StudioContext";
import { type Mounted, mount } from "./dom";

const transform = { x: 0, y: 0, width: 1, height: 1, opacity: 1 };
const srt = (url: string, extra: Partial<Source> = {}): Source =>
  ({ id: "s1", name: "SRT", visible: true, transform, volume: 1, muted: false, kind: "srt_input", url, ...extra }) as Source;

describe("réglages de capture", () => {
  it("seuls le type et ses champs comptent", () => {
    const a = srt("srt://0.0.0.0:9000?mode=listener");
    expect(captureSettingsKey({ ...a, name: "Autre", visible: false, volume: 0.5, muted: true })).toBe(captureSettingsKey(a));
    expect(captureSettingsKey(srt("srt://0.0.0.0:9100?mode=listener"))).not.toBe(captureSettingsKey(a));
    expect(captureSettingsKey({ ...a, kind: "color", rgba: [0, 0, 0, 255] } as Source)).toBeNull();
  });

  it("relance, retrait ou rien", () => {
    expect(captureSettingsChange("a", "b")).toBe("restarted");
    expect(captureSettingsChange(null, "b")).toBe("restarted");
    expect(captureSettingsChange("a", null)).toBe("removed");
    expect(captureSettingsChange("a", "a")).toBe("unchanged");
    expect(captureSettingsChange(undefined, "a")).toBe("unchanged");
  });
});

/** Client factice : instantanés fixes, événements poussés par le test. */
function fakeClient(studio: StudioState) {
  let handler: ((data: unknown, ev: { type: string }) => void) | null = null;
  const client = {
    state: "connected",
    info: null,
    onState: () => () => {},
    subscribe: (_type: string, h: typeof handler) => {
      handler = h;
      return () => {
        handler = null;
      };
    },
    call: async (type: string) => {
      switch (type) {
        case "GetStudio":
          return studio;
        case "GetCaptureStatus":
          return { sources: [{ source_id: "s1", kind: "srt_input", state: "running" }] };
        case "GetOutputStatus":
          return { backend: "gstreamer", record: { active: false }, stream: { active: false } };
        default:
          throw new Error(type);
      }
    },
  };
  const emit = (type: string, data: unknown) => act(() => handler?.(data, { type }));
  return { client: client as unknown as StudioClient, emit };
}

describe("état des captures après UpdateSource", () => {
  let m: Mounted | null = null;
  afterEach(() => {
    m?.unmount();
    m = null;
  });

  it("de nouveaux réglages repartent en démarrage et notifient leur premier échec", async () => {
    const source = srt("srt://0.0.0.0:9000?mode=listener");
    const studio = {
      scenes: [{ id: "sc", name: "Scène", sources: [source] }],
      program_scene: "sc",
      preview_scene: "sc",
      canvas: { width: 1280, height: 720, fps: 30 },
    } as unknown as StudioState;
    const { client, emit } = fakeClient(studio);
    let ctx: StudioContextValue | null = null;
    function Probe() {
      ctx = useStudio();
      return null;
    }
    m = mount(
      <StudioProvider client={client} url="ws://test">
        <Probe />
      </StudioProvider>,
    );
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(ctx!.captures.s1).toEqual({ state: "running" });

    // Renommer ne relance pas la capture.
    emit("SourceUpdated", { scene_id: "sc", source: { ...source, name: "Renommée" } });
    expect(ctx!.captures.s1).toEqual({ state: "running" });

    // Échec, puis nouvelle URL : l'erreur d'avant ne reste pas affichée.
    emit("SourceFailed", { source_id: "s1", kind: "srt_input", error: "port pris", retry_in_ms: 1000 });
    expect(ctx!.toasts).toHaveLength(1);
    emit("SourceUpdated", { scene_id: "sc", source: srt("srt://0.0.0.0:9100?mode=listener") });
    expect(ctx!.captures.s1).toEqual({ state: "starting" });

    // Le premier échec des nouveaux réglages est notifié.
    emit("SourceFailed", { source_id: "s1", kind: "srt_input", error: "adresse invalide", retry_in_ms: 1000 });
    expect(ctx!.toasts).toHaveLength(2);
    expect(ctx!.captures.s1?.state).toBe("failed");

    // Devenue une couleur : plus de capture.
    emit("SourceUpdated", { scene_id: "sc", source: { ...source, kind: "color", rgba: [0, 0, 0, 255] } });
    expect(ctx!.captures.s1).toBeUndefined();
  });
});
