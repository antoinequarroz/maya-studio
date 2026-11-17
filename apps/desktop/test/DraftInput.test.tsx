// @vitest-environment jsdom
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DraftInput } from "../src/components/DraftInput";
import { type Mounted, mount, press, typeInto } from "./dom";

let m: Mounted | null = null;
afterEach(() => {
  m?.unmount();
  m = null;
});

function setup(value = "10") {
  const onCommit = vi.fn();
  m = mount(<DraftInput value={value} onCommit={onCommit} />);
  const input = m.container.querySelector("input")!;
  act(() => input.focus());
  return { input, onCommit };
}

describe("DraftInput", () => {
  it("Échap annule le brouillon sans rien valider", () => {
    const { input, onCommit } = setup();
    typeInto(input, "55");
    press(input, "Escape");
    expect(onCommit).not.toHaveBeenCalled();
    expect(input.value).toBe("10");
    expect(document.activeElement).not.toBe(input);
  });

  it("Entrée valide une seule fois", () => {
    const { input, onCommit } = setup();
    typeInto(input, "77");
    press(input, "Enter");
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledWith("77");
  });

  it("la sortie du champ valide une seule fois", () => {
    const { input, onCommit } = setup();
    typeInto(input, "12");
    act(() => input.blur());
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledWith("12");
  });

  it("ne valide rien si la valeur n'a pas changé", () => {
    const { input, onCommit } = setup();
    typeInto(input, "10");
    press(input, "Enter");
    expect(onCommit).not.toHaveBeenCalled();
  });
});
