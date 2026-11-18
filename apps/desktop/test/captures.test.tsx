// @vitest-environment jsdom
import type { EncodersInfo, InputDevice, Source } from "@suite-media/client";
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  dbToFraction,
  formatGainDb,
  loudest,
  METER_FLOOR_DB,
  PEAK_HOLD_MS,
  percentToVolume,
  silentChannel,
  stepChannel,
  volumeToPercent,
} from "../src/audio/meter";
import { DevicePicker, uniqueName } from "../src/components/AddSourceDialog";
import { describeEncoder } from "../src/components/OutputsPanel";
import { baseName, KIND_LABELS, sourceSummary } from "../src/sourceKinds";
import { type Mounted, mount } from "./dom";

describe("VU-mètres", () => {
  it("échelle linéaire en dB entre -60 et 0 dBFS", () => {
    expect(dbToFraction(-100)).toBe(0);
    expect(dbToFraction(METER_FLOOR_DB)).toBe(0);
    expect(dbToFraction(-30)).toBeCloseTo(0.5);
    expect(dbToFraction(0)).toBe(1);
    expect(dbToFraction(3)).toBe(1);
    expect(dbToFraction(Number.NaN)).toBe(0);
  });

  it("montée immédiate, descente progressive, crête maintenue puis relâchée", () => {
    let ch = stepChannel(silentChannel(), { rms: -12, peak: -6 }, 16, 0);
    expect(ch.rms).toBe(-12);
    expect(ch.peak).toBe(-6);
    expect(ch.hold).toBe(-6);
    // Silence : les barres descendent doucement (pas de saut au plancher).
    ch = stepChannel(ch, { rms: -100, peak: -100 }, 100, 100);
    expect(ch.rms).toBeLessThan(-12);
    expect(ch.rms).toBeGreaterThan(-16);
    expect(ch.hold).toBe(-6);
    // Après le maintien, la crête redescend aussi.
    ch = stepChannel(ch, { rms: -100, peak: -100 }, 100, PEAK_HOLD_MS + 200);
    expect(ch.hold).toBeLessThan(-6);
    // Jamais sous le plancher.
    for (let t = 0; t < 100; t++) ch = stepChannel(ch, { rms: -100, peak: -100 }, 100, PEAK_HOLD_MS + 300 + t * 100);
    expect(ch.rms).toBe(METER_FLOOR_DB);
    expect(ch.hold).toBe(METER_FLOOR_DB);
  });

  it("canal le plus fort", () => {
    expect(loudest([-12, -6.5])).toBe(-6.5);
    expect(loudest([])).toBe(METER_FLOOR_DB);
    expect(loudest(undefined)).toBe(METER_FLOOR_DB);
  });

  it("volume : pourcentage du curseur et gain en dB", () => {
    expect(volumeToPercent(undefined)).toBe(100);
    expect(volumeToPercent(1.5)).toBe(150);
    expect(percentToVolume(250)).toBe(2);
    expect(percentToVolume(-5)).toBe(0);
    expect(percentToVolume(73)).toBe(0.73);
    expect(formatGainDb(1)).toBe("0,0 dB");
    expect(formatGainDb(2)).toBe("+6,0 dB");
    expect(formatGainDb(0.5)).toBe("−6,0 dB");
    expect(formatGainDb(0)).toBe("−∞ dB");
  });
});

describe("sources de capture", () => {
  const base = { id: "x", name: "x", visible: true, transform: { x: 0, y: 0, width: 1, height: 1, opacity: 1 } };

  it("un libellé français pour chaque type", () => {
    expect(Object.keys(KIND_LABELS)).toHaveLength(9);
    expect(KIND_LABELS.display_capture).toBe("Écran");
  });

  it("résumés lisibles", () => {
    const names: Record<string, string> = { "alsa_input.usb-Focusrite": "Scarlett 2i2" };
    const mic: Source = { ...base, kind: "audio_input", device_id: "alsa_input.usb-Focusrite" };
    expect(sourceSummary(mic, (id) => names[id])).toBe("Scarlett 2i2");
    expect(sourceSummary({ ...base, kind: "audio_input" })).toBe("Entrée audio par défaut");
    expect(sourceSummary({ ...base, kind: "media_file", path: "/v/clip.mp4", loop: true })).toBe("/v/clip.mp4 (en boucle)");
    expect(sourceSummary({ ...base, kind: "srt_input", url: "srt://h:9000" })).toBe("srt://h:9000");
    expect(baseName("C:\\Vidéos\\clip.mkv")).toBe("clip.mkv");
    expect(baseName("file:///tmp/a/b.mp4?x=1")).toBe("b.mp4");
  });

  it("noms par défaut uniques", () => {
    expect(uniqueName("Caméra", [])).toBe("Caméra");
    expect(uniqueName("Caméra", ["Caméra", "Caméra 2"])).toBe("Caméra 3");
  });
});

describe("encodeur affiché", () => {
  const encoders: EncodersInfo = {
    backend: "gstreamer",
    requested: "auto",
    encoders: [
      { name: "vtenc_h264_hw", hardware: true, available: true, selected: true, gpl: false },
      { name: "openh264enc", hardware: false, available: true, selected: false, gpl: false },
    ],
  };

  it("celui de la sortie active, sinon celui que le moteur choisira", () => {
    expect(describeEncoder({ active: false }, encoders)).toEqual({ name: "vtenc_h264_hw", kind: "matériel" });
    expect(describeEncoder({ active: true, encoder: "openh264enc" }, encoders)).toEqual({ name: "openh264enc", kind: "logiciel" });
    expect(describeEncoder({ active: true, encoder: "libx264" }, null)).toEqual({ name: "libx264", kind: null });
    expect(describeEncoder({ active: false }, null)).toBeNull();
  });
});

describe("DevicePicker", () => {
  let m: Mounted | null = null;
  afterEach(() => {
    m?.unmount();
    m = null;
  });

  const scarlett: InputDevice = {
    id: "alsa_input.usb-Focusrite-00",
    name: "Scarlett 2i2",
    caps_summary: "audio/x-raw (S32LE) · 2 canaux · 48000 Hz",
  };

  it("liste « par défaut » puis les périphériques du moteur", () => {
    const onChange = vi.fn();
    m = mount(
      <DevicePicker label="Entrée audio" devices={[scarlett]} value="" loading={false} onChange={onChange} onReload={() => {}} emptyText="vide" />,
    );
    const select = m.container.querySelector("select")!;
    expect([...select.options].map((o) => o.textContent)).toEqual(["Par défaut du système", "Scarlett 2i2"]);
    act(() => {
      select.value = scarlett.id;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(onChange).toHaveBeenCalledWith(scarlett.id);
  });

  it("garde un périphérique absent et affiche ses formats quand il est là", () => {
    m = mount(
      <DevicePicker label="Caméra" devices={[]} value="/dev/video9" loading={false} onChange={() => {}} onReload={() => {}} emptyText="Aucune caméra" />,
    );
    expect(m.container.textContent).toContain("Aucune caméra");
    expect([...m.container.querySelectorAll("option")].map((o) => o.textContent)).toContain("/dev/video9 (absent)");
    m.rerender(
      <DevicePicker label="Entrée" devices={[scarlett]} value={scarlett.id} loading={false} onChange={() => {}} onReload={() => {}} emptyText="" />,
    );
    expect(m.container.textContent).toContain("2 canaux");
  });
});
