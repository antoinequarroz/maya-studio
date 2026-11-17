/** Connexion au studio et détection des erreurs qui l'arrêtent pour de bon. */

import { isSuiteError, type SuiteClient } from "@suite-media/client";

/** Message à montrer pour une erreur qui arrête définitivement la connexion, sinon `null`. */
export function fatalMessage(e: unknown): string | null {
  if (isSuiteError(e, "auth_failed")) return "Jeton refusé par le studio.";
  if (isSuiteError(e, "protocol_mismatch")) return `Studio incompatible : ${e.message}`;
  return null;
}

/**
 * Lance la connexion et appelle `onFatal` quand le client abandonne : à la première
 * connexion (promesse de `connect()`), mais aussi plus tard, quand une reconnexion
 * échoue sur une erreur fatale (studio externe redémarré avec un autre jeton ou une
 * autre version du protocole) ou que les tentatives sont épuisées.
 * Renvoie la fonction de nettoyage, qui ferme le client sans appeler `onFatal`.
 */
export function connectWatchingFatal(client: SuiteClient, onFatal: (message: string) => void): () => void {
  let fatal: string | null = null;
  let wasConnected = false;
  const offError = client.onError((e) => {
    fatal = fatalMessage(e) ?? fatal;
  });
  const offState = client.onState((st) => {
    if (st === "connected") wasConnected = true;
    else if (st === "closed" && wasConnected) onFatal(fatal ?? "Connexion au studio fermée : le moteur ne répond plus.");
  });
  client.connect().catch((e: unknown) => {
    // Après une première connexion réussie, c'est l'écouteur d'état qui s'en charge.
    if (wasConnected) return;
    const msg = fatalMessage(e);
    if (msg) onFatal(msg);
    else if (!isSuiteError(e, "not_connected")) onFatal(`Connexion impossible : ${String((e as Error).message ?? e)}`);
  });
  return () => {
    offError();
    offState();
    client.close();
  };
}
