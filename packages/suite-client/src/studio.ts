/**
 * Domaines de Maya Studio (`app.id = "studio"`) : types des requêtes, réponses
 * et événements, miroir de `crates/studiod/src/api.rs` et `crates/studio-core`.
 *
 * Toute évolution côté Rust doit être reportée ici (ajouts seulement, le
 * protocole reste rétrocompatible).
 */

import { SuiteClient, type RequestOptions, type SuiteClientOptions } from "./client.js";
import type { AppInfo, EventMessage } from "./protocol.js";

export const STUDIO_APP_ID = "studio";

/** Identifiant UUID v4 en texte. */
export type Uuid = string;

/** Taille et cadence de la sortie programme. */
export interface Canvas {
  width: number;
  height: number;
  fps: number;
}

/** Position et taille d'une source en pixels du canvas ; opacité de 0 à 1 (1 par défaut). */
export interface Transform {
  x: number;
  y: number;
  width: number;
  height: number;
  opacity?: number;
}

/** Couleur RGBA, chaque composante de 0 à 255. */
export type Rgba = [number, number, number, number];

/** Type de source et ses réglages (champ `kind`). */
export type SourceKind =
  | { kind: "color"; rgba: Rgba }
  | { kind: "image"; path: string }
  | { kind: "test_pattern" };

export type SourceKindName = SourceKind["kind"];

export type Source = {
  id: Uuid;
  name: string;
  transform: Required<Transform>;
  visible: boolean;
} & SourceKind;

/** Une scène : pile ordonnée de sources, la première est dessinée en dessous. */
export interface Scene {
  id: Uuid;
  name: string;
  sources: Source[];
}

/** État complet du studio (réponse de `GetStudio`). */
export interface StudioState {
  canvas: Canvas;
  scenes: Scene[];
  program_scene: Uuid;
  preview_scene: Uuid;
}

export interface SceneSummary {
  id: Uuid;
  name: string;
  /** Nombre de sources. */
  sources: number;
}

export type OutputKind = "record" | "stream";

export interface OutputState {
  active: boolean;
  target?: string;
  frames?: number;
  duration_s?: number;
}

export interface OutputStatus {
  record: OutputState;
  stream: OutputState;
}

export interface OutputStartedData {
  output: OutputKind;
  target: string;
}

export interface OutputStoppedData {
  output: OutputKind;
  target: string;
  frames: number;
  duration_s: number;
}

export interface OutputFailedData {
  output: OutputKind;
  error: string;
}

/** Données de `UpdateSource` : seuls les champs présents changent. */
export type SourcePatch = {
  name?: string;
  visible?: boolean;
  transform?: Transform;
} & (SourceKind | { kind?: undefined });

/** Spécification d'une requête : données envoyées et données de la réponse. */
export interface RequestSpec<D, R> {
  data: D;
  result: R;
}

/** Toutes les requêtes du studio, par type. `void` = pas de données. */
export interface StudioRequests {
  // core
  GetAppInfo: RequestSpec<void, { app: AppInfo; protocol: number; capabilities: string[] }>;
  GetRequestList: RequestSpec<void, { requests: string[] }>;
  Ping: RequestSpec<void, { pong: true }>;
  // studio.scenes
  GetStudio: RequestSpec<void, StudioState>;
  GetSceneList: RequestSpec<void, { scenes: SceneSummary[]; program_scene: Uuid; preview_scene: Uuid }>;
  GetScene: RequestSpec<{ scene: string }, Scene>;
  CreateScene: RequestSpec<{ name: string }, { scene: Scene }>;
  RemoveScene: RequestSpec<{ scene: string }, { scene_id: Uuid }>;
  RenameScene: RequestSpec<{ scene: string; name: string }, { scene_id: Uuid; name: string }>;
  SetProgramScene: RequestSpec<{ scene: string }, { scene_id: Uuid }>;
  SetPreviewScene: RequestSpec<{ scene: string }, { scene_id: Uuid }>;
  Cut: RequestSpec<void, Record<string, never>>;
  SetCanvas: RequestSpec<Canvas, Canvas>;
  // studio.sources
  AddSource: RequestSpec<
    { scene: string; name: string; transform?: Transform } & SourceKind,
    { scene_id: Uuid; source: Source }
  >;
  RemoveSource: RequestSpec<{ scene: string; source: string }, { scene_id: Uuid; source_id: Uuid }>;
  UpdateSource: RequestSpec<{ scene: string; source: string } & SourcePatch, { scene_id: Uuid; source: Source }>;
  MoveSource: RequestSpec<{ scene: string; source: string; index: number }, { scene_id: Uuid; source_ids: Uuid[] }>;
  // studio.preview
  GetScreenshot: RequestSpec<{ scene?: string; format?: "png" | "jpeg" } | void, { image: string }>;
  // studio.outputs
  StartRecording: RequestSpec<{ path?: string } | void, OutputStartedData>;
  StopRecording: RequestSpec<void, OutputStoppedData>;
  StartStream: RequestSpec<{ url: string }, OutputStartedData>;
  StopStream: RequestSpec<void, OutputStoppedData>;
  GetOutputStatus: RequestSpec<void, OutputStatus>;
}

export type StudioRequestType = keyof StudioRequests;

/** Tous les événements du studio, par type. */
export interface StudioEvents {
  SceneCreated: { scene: Scene };
  SceneRemoved: { scene_id: Uuid };
  SceneRenamed: { scene_id: Uuid; name: string };
  ProgramSceneChanged: { scene_id: Uuid };
  PreviewSceneChanged: { scene_id: Uuid };
  CanvasChanged: Canvas;
  SourceAdded: { scene_id: Uuid; source: Source };
  SourceRemoved: { scene_id: Uuid; source_id: Uuid };
  SourceUpdated: { scene_id: Uuid; source: Source };
  SourceReordered: { scene_id: Uuid; source_ids: Uuid[] };
  OutputStarted: OutputStartedData;
  OutputStopped: OutputStoppedData;
  OutputFailed: OutputFailedData;
}

export type StudioEventType = keyof StudioEvents;

/** Domaine nécessaire à chaque requête (miroir de `REQUESTS` / `CAPABILITIES` dans api.rs). */
export const STUDIO_REQUEST_CAPABILITIES: Readonly<Record<StudioRequestType, string>> = Object.freeze({
  GetAppInfo: "core",
  GetRequestList: "core",
  Ping: "core",
  GetStudio: "studio.scenes",
  GetSceneList: "studio.scenes",
  GetScene: "studio.scenes",
  CreateScene: "studio.scenes",
  RemoveScene: "studio.scenes",
  RenameScene: "studio.scenes",
  SetProgramScene: "studio.scenes",
  SetPreviewScene: "studio.scenes",
  Cut: "studio.scenes",
  SetCanvas: "studio.scenes",
  AddSource: "studio.sources",
  RemoveSource: "studio.sources",
  UpdateSource: "studio.sources",
  MoveSource: "studio.sources",
  GetScreenshot: "studio.preview",
  StartRecording: "studio.outputs",
  StopRecording: "studio.outputs",
  StartStream: "studio.outputs",
  StopStream: "studio.outputs",
  GetOutputStatus: "studio.outputs",
});

/** Liste des requêtes connues, dans l'ordre de `REQUESTS` (api.rs). */
export const STUDIO_REQUESTS = Object.keys(STUDIO_REQUEST_CAPABILITIES) as StudioRequestType[];

export const STUDIO_EVENTS: readonly StudioEventType[] = Object.freeze([
  "SceneCreated",
  "SceneRemoved",
  "SceneRenamed",
  "ProgramSceneChanged",
  "PreviewSceneChanged",
  "CanvasChanged",
  "SourceAdded",
  "SourceRemoved",
  "SourceUpdated",
  "SourceReordered",
  "OutputStarted",
  "OutputStopped",
  "OutputFailed",
]);

/** Codes d'erreur propres au studio. */
export const StudioErrorCode = {
  SceneNotFound: "scene_not_found",
  SourceNotFound: "source_not_found",
  SceneNameTaken: "scene_name_taken",
  SceneOnAir: "scene_on_air",
  LastScene: "last_scene",
  OutputsActive: "outputs_active",
  OutputAlreadyActive: "output_already_active",
  OutputNotActive: "output_not_active",
  MissingUrl: "missing_url",
  OutputError: "output_error",
  ScreenshotFailed: "screenshot_failed",
} as const;

/**
 * Requêtes dont tous les champs sont optionnels : le serveur exige quand même un
 * objet `data` (éventuellement vide), le client envoie donc `{}` par défaut.
 */
const OPTIONAL_DATA_REQUESTS: ReadonlySet<string> = new Set(["GetScreenshot", "StartRecording"]);

/** Arguments de données : optionnels si la requête n'en prend pas. */
export type DataArgs<D> = [D] extends [void]
  ? [data?: undefined, options?: RequestOptions]
  : undefined extends D
    ? [data?: D, options?: RequestOptions]
    : void extends D
      ? [data?: Exclude<D, void>, options?: RequestOptions]
      : [data: D, options?: RequestOptions];

/** Client du studio : le client générique, plus des requêtes et événements typés. */
export class StudioClient extends SuiteClient {
  constructor(options: SuiteClientOptions) {
    super({
      expectedApp: STUDIO_APP_ID,
      requestCapabilities: STUDIO_REQUEST_CAPABILITIES,
      ...options,
    });
  }

  /** Requête typée : `await studio.call("CreateScene", { name: "Plateau" })`. */
  call<K extends StudioRequestType>(
    type: K,
    ...args: DataArgs<StudioRequests[K]["data"]>
  ): Promise<StudioRequests[K]["result"]> {
    const [given, options] = args;
    const data = given === undefined && OPTIONAL_DATA_REQUESTS.has(type) ? {} : given;
    return this.request<StudioRequests[K]["result"]>(type, data, options);
  }

  /** Abonnement typé à un événement du studio (`*` pour tous). */
  subscribe<K extends StudioEventType>(
    type: K,
    handler: (data: StudioEvents[K], event: EventMessage) => void,
  ): () => void;
  subscribe(type: "*", handler: (data: unknown, event: EventMessage) => void): () => void;
  subscribe(type: string, handler: (data: never, event: EventMessage) => void): () => void {
    return this.on(STUDIO_APP_ID, type, handler as (data: unknown, event: EventMessage) => void);
  }
}

// --- état local tenu à jour par les événements ----------------------------

function mapScene(state: StudioState, sceneId: Uuid, f: (scene: Scene) => Scene): StudioState {
  let found = false;
  const scenes = state.scenes.map((s) => {
    if (s.id !== sceneId) return s;
    found = true;
    return f(s);
  });
  return found ? { ...state, scenes } : state;
}

/**
 * Applique un événement du studio à un état local, sans le muter.
 * Renvoie le même objet si l'événement ne change rien (ou est inconnu),
 * ce qui permet aux UI de ne pas se redessiner pour rien.
 */
export function applyStudioEvent(state: StudioState, type: string, data: unknown): StudioState {
  switch (type as StudioEventType) {
    case "SceneCreated": {
      const { scene } = data as StudioEvents["SceneCreated"];
      if (state.scenes.some((s) => s.id === scene.id)) return state;
      return { ...state, scenes: [...state.scenes, scene] };
    }
    case "SceneRemoved": {
      const { scene_id } = data as StudioEvents["SceneRemoved"];
      if (!state.scenes.some((s) => s.id === scene_id)) return state;
      return {
        ...state,
        scenes: state.scenes.filter((s) => s.id !== scene_id),
        // Même règle que le moteur : l'aperçu retombe sur le programme.
        preview_scene: state.preview_scene === scene_id ? state.program_scene : state.preview_scene,
      };
    }
    case "SceneRenamed": {
      const { scene_id, name } = data as StudioEvents["SceneRenamed"];
      return mapScene(state, scene_id, (s) => ({ ...s, name }));
    }
    case "ProgramSceneChanged": {
      const { scene_id } = data as StudioEvents["ProgramSceneChanged"];
      return state.program_scene === scene_id ? state : { ...state, program_scene: scene_id };
    }
    case "PreviewSceneChanged": {
      const { scene_id } = data as StudioEvents["PreviewSceneChanged"];
      return state.preview_scene === scene_id ? state : { ...state, preview_scene: scene_id };
    }
    case "CanvasChanged":
      return { ...state, canvas: { ...(data as Canvas) } };
    case "SourceAdded": {
      const { scene_id, source } = data as StudioEvents["SourceAdded"];
      return mapScene(state, scene_id, (s) =>
        s.sources.some((x) => x.id === source.id) ? s : { ...s, sources: [...s.sources, source] },
      );
    }
    case "SourceRemoved": {
      const { scene_id, source_id } = data as StudioEvents["SourceRemoved"];
      return mapScene(state, scene_id, (s) => ({ ...s, sources: s.sources.filter((x) => x.id !== source_id) }));
    }
    case "SourceUpdated": {
      const { scene_id, source } = data as StudioEvents["SourceUpdated"];
      return mapScene(state, scene_id, (s) => ({
        ...s,
        sources: s.sources.map((x) => (x.id === source.id ? source : x)),
      }));
    }
    case "SourceReordered": {
      const { scene_id, source_ids } = data as StudioEvents["SourceReordered"];
      return mapScene(state, scene_id, (s) => {
        const byId = new Map(s.sources.map((x) => [x.id, x]));
        const ordered = source_ids.map((id) => byId.get(id)).filter((x): x is Source => x !== undefined);
        // Sources inconnues de la liste (ne devrait pas arriver) : gardées au-dessus.
        const rest = s.sources.filter((x) => !source_ids.includes(x.id));
        return { ...s, sources: [...ordered, ...rest] };
      });
    }
    default:
      return state;
  }
}

/** Recherche pratique d'une scène par identifiant. */
export function findScene(state: StudioState, sceneId: Uuid): Scene | undefined {
  return state.scenes.find((s) => s.id === sceneId);
}
