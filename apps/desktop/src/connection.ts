/**
 * Où se trouve le studio ? Le front-end ne parle au moteur qu'en WebSocket :
 * seule l'adresse et le jeton changent selon l'environnement.
 *
 * 1. Dans Tauri : le shell a lancé `studiod` en sidecar et donne URL + jeton
 *    par la commande `studio_connection`.
 * 2. Dans un navigateur : paramètres `?url=…&token=…` de la page.
 * 3. En développement : variables `VITE_STUDIO_URL` et `VITE_STUDIO_TOKEN`.
 * 4. Sinon : écran de connexion (studio distant).
 */

import { invoke, isTauri } from "@tauri-apps/api/core";

export interface StudioTarget {
  url: string;
  token?: string;
  /** D'où vient la configuration, pour l'affichage. */
  origin: "tauri" | "url" | "env" | "manual";
}

interface TauriConnection {
  url: string;
  token: string | null;
  managed: boolean;
}

const LAST_URL_KEY = "maya-studio.last-url";

export async function resolveStudioTarget(): Promise<StudioTarget | null> {
  if (isTauri()) {
    const c = await invoke<TauriConnection>("studio_connection");
    return { url: c.url, token: c.token ?? undefined, origin: "tauri" };
  }
  const params = new URLSearchParams(window.location.search);
  const url = params.get("url");
  if (url) return { url, token: params.get("token") ?? undefined, origin: "url" };
  const envUrl = import.meta.env.VITE_STUDIO_URL;
  if (envUrl) return { url: envUrl, token: import.meta.env.VITE_STUDIO_TOKEN || undefined, origin: "env" };
  return null;
}

export function lastUrl(): string {
  try {
    return localStorage.getItem(LAST_URL_KEY) ?? "ws://127.0.0.1:4460";
  } catch {
    return "ws://127.0.0.1:4460";
  }
}

export function rememberUrl(url: string): void {
  try {
    localStorage.setItem(LAST_URL_KEY, url);
  } catch {
    // stockage indisponible : sans conséquence
  }
}
