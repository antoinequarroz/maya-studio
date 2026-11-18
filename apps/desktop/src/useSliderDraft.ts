/** Valeur affichée d'un curseur réglé à distance (volume…), sans retour en arrière. */

import { useEffect, useRef, useState } from "react";

/** Délai après lequel une valeur relâchée jamais confirmée cède la place à celle du serveur. */
export const DRAFT_SETTLE_MS = 1500;

/**
 * Garde la valeur choisie par l'utilisateur tant que le serveur ne l'a pas confirmée.
 *
 * Pendant un glissé, la valeur locale fait foi. Une fois le curseur relâché (ou après
 * un réglage au clavier ou au double-clic), elle reste affichée jusqu'à ce que
 * `server` l'atteigne, au plus {@link DRAFT_SETTLE_MS} : le dernier envoi, différé
 * ou pas encore confirmé, ne fait donc pas revenir le curseur en arrière, et un
 * changement venu d'ailleurs redevient visible dès la confirmation.
 */
export function useSliderDraft(server: number) {
  const [draft, setDraft] = useState<{ value: number; settled: boolean } | null>(null);
  const dragging = useRef(false);

  // Confirmation reçue : la valeur du serveur reprend la main.
  useEffect(() => {
    if (draft?.settled && draft.value === server) setDraft(null);
  }, [draft, server]);

  // Pas de confirmation (requête refusée, valeur bornée par le serveur) : on n'insiste pas.
  useEffect(() => {
    if (!draft?.settled) return;
    const t = window.setTimeout(() => setDraft(null), DRAFT_SETTLE_MS);
    return () => window.clearTimeout(t);
  }, [draft]);

  return {
    value: draft?.value ?? server,
    /** Nouvelle valeur choisie (glissé, clavier, double-clic). */
    set: (value: number) => setDraft({ value, settled: !dragging.current }),
    /** Début d'un glissé à la souris ou au doigt. */
    grab: () => {
      dragging.current = true;
    },
    /** Fin du glissé (ou perte du focus) : on attend la confirmation du serveur. */
    release: () => {
      dragging.current = false;
      setDraft((d) => (d && !d.settled ? { ...d, settled: true } : d));
    },
  };
}
