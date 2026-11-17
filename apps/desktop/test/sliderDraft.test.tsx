// @vitest-environment jsdom
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DRAFT_SETTLE_MS, useSliderDraft } from "../src/useSliderDraft";
import { type Mounted, mount } from "./dom";

type Draft = ReturnType<typeof useSliderDraft>;

describe("useSliderDraft", () => {
  let m: Mounted | null = null;
  let draft: Draft;
  function Probe({ server }: { server: number }) {
    draft = useSliderDraft(server);
    return null;
  }
  afterEach(() => {
    m?.unmount();
    m = null;
    vi.useRealTimers();
  });

  it("garde la valeur relâchée jusqu'à la confirmation du serveur", () => {
    m = mount(<Probe server={100} />);
    act(() => draft.grab());
    act(() => draft.set(120));
    act(() => draft.set(140));
    // Le serveur n'a confirmé qu'une valeur intermédiaire.
    m.rerender(<Probe server={120} />);
    expect(draft.value).toBe(140);
    act(() => draft.release());
    expect(draft.value).toBe(140);
    m.rerender(<Probe server={140} />);
    expect(draft.value).toBe(140);
    // Confirmée : un changement venu d'ailleurs redevient visible.
    m.rerender(<Probe server={90} />);
    expect(draft.value).toBe(90);
  });

  it("un réglage au clavier n'attend pas la perte du focus", () => {
    m = mount(<Probe server={100} />);
    act(() => draft.set(101));
    expect(draft.value).toBe(101);
    m.rerender(<Probe server={101} />);
    m.rerender(<Probe server={50} />);
    expect(draft.value).toBe(50);
  });

  it("une valeur jamais confirmée cède au bout du délai", () => {
    vi.useFakeTimers();
    m = mount(<Probe server={100} />);
    act(() => draft.set(150));
    act(() => vi.advanceTimersByTime(DRAFT_SETTLE_MS - 10));
    expect(draft.value).toBe(150);
    act(() => vi.advanceTimersByTime(20));
    expect(draft.value).toBe(100);
  });
});
