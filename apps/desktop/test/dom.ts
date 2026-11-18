/** Petits utilitaires de test DOM (jsdom + React 19), sans dépendance supplémentaire. */

import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

export interface Mounted {
  container: HTMLElement;
  root: Root;
  rerender: (node: ReactNode) => void;
  unmount: () => void;
}

export function mount(node: ReactNode): Mounted {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  return {
    container,
    root,
    rerender: (n) => act(() => root.render(n)),
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

/** Simule une saisie : React écoute l'événement `input` et compare à la valeur native. */
export function typeInto(input: HTMLInputElement, value: string): void {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

export function press(target: Element, key: string): KeyboardEvent {
  const ev = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  act(() => {
    target.dispatchEvent(ev);
  });
  return ev;
}
