/** Réglages de la reconnexion automatique. */
export interface BackoffOptions {
  /** Premier délai avant reconnexion, en ms (défaut 500). */
  initialDelayMs?: number;
  /** Délai maximal, en ms (défaut 10 000). */
  maxDelayMs?: number;
  /** Multiplicateur entre deux tentatives (défaut 2). */
  factor?: number;
  /** Part aléatoire du délai, de 0 à 1 (défaut 0.2), pour éviter que tous les clients reviennent en même temps. */
  jitter?: number;
  /** Nombre maximal de tentatives consécutives (défaut : illimité). */
  maxAttempts?: number;
}

export const DEFAULT_BACKOFF: Required<BackoffOptions> = {
  initialDelayMs: 500,
  maxDelayMs: 10_000,
  factor: 2,
  jitter: 0.2,
  maxAttempts: Number.POSITIVE_INFINITY,
};

/**
 * Délai avant la tentative `attempt` (0 = première reconnexion).
 * Exponentiel borné, avec une part aléatoire en moins (le délai ne dépasse jamais `maxDelayMs`).
 */
export function backoffDelay(attempt: number, options: BackoffOptions = {}, random: () => number = Math.random): number {
  const o = { ...DEFAULT_BACKOFF, ...options };
  const base = Math.min(o.maxDelayMs, o.initialDelayMs * Math.pow(o.factor, Math.max(0, attempt)));
  const jitter = Math.min(1, Math.max(0, o.jitter));
  return Math.round(base * (1 - jitter * random()));
}
