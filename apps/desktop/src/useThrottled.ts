/** Envoi limité pour les gestes continus (curseurs, nuancier). */

import { useEffect, useRef } from "react";

/** Limite un envoi continu (curseurs, nuancier) à une requête toutes les `ms`, la dernière valeur gagne. */
export function useThrottled<T>(send: (v: T) => void, ms = 80) {
  const last = useRef(0);
  const pending = useRef<{ v: T } | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const sendRef = useRef(send);
  sendRef.current = send;
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return (v: T) => {
    pending.current = { v };
    const wait = last.current + ms - performance.now();
    if (wait <= 0) {
      last.current = performance.now();
      sendRef.current(v);
      pending.current = null;
    } else if (timer.current === undefined) {
      timer.current = window.setTimeout(() => {
        timer.current = undefined;
        last.current = performance.now();
        if (pending.current) sendRef.current(pending.current.v);
        pending.current = null;
      }, wait);
    }
  };
}
