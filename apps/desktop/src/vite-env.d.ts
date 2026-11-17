/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** URL WebSocket du studio en mode développement sans Tauri, ex. ws://127.0.0.1:4460. */
  readonly VITE_STUDIO_URL?: string;
  /** Jeton du studio en mode développement sans Tauri. */
  readonly VITE_STUDIO_TOKEN?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
