/** Choix de la source sélectionnée quand la scène change. */

/**
 * Source tout juste ajoutée et sélectionnée, pas encore présente dans la scène :
 * la réponse à `AddSource` peut précéder l'événement `SourceAdded`.
 */
export interface PendingSelection {
  id: string;
  /** Instant (ms, `Date.now()`) au-delà duquel on n'attend plus l'événement. */
  until: number;
}

/** Délai d'attente de l'événement `SourceAdded` après la réponse. */
export const PENDING_SELECTION_MS = 2000;

/**
 * Source à sélectionner : la sélection actuelle si elle existe dans la scène,
 * ou si elle est en attente de son événement d'ajout ; sinon la plus haute.
 */
export function reconcileSelection(
  sourceIds: readonly string[],
  selected: string | null,
  pending: PendingSelection | null,
  now: number,
): string | null {
  if (selected && sourceIds.includes(selected)) return selected;
  if (selected && pending && pending.id === selected && now < pending.until) return selected;
  return sourceIds.at(-1) ?? null;
}
