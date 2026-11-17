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

/**
 * Type de source et ses réglages (champ `kind`), miroir de `SourceKind` (studio-core).
 *
 * Les sources de capture (`camera` à `srt_input`) sont alimentées par le moteur ;
 * leur état arrive par `SourceStarted` / `SourceFailed`.
 */
export type SourceKind =
  | { kind: "color"; rgba: Rgba }
  | { kind: "image"; path: string }
  | { kind: "test_pattern" }
  /** Caméra ; sans `device_id`, la caméra par défaut. Mode souhaité optionnel. */
  | { kind: "camera"; device_id?: string; width?: number; height?: number; fps?: number }
  /** Écran entier ; sans `display_id`, l'écran principal. */
  | { kind: "display_capture"; display_id?: string }
  /** Fenêtre (selon la plateforme). */
  | { kind: "window_capture"; window_id?: string }
  /** Micro ou interface audio ; sans `device_id`, l'entrée par défaut. */
  | { kind: "audio_input"; device_id?: string }
  /** Fichier vidéo ou audio (chemin local ou URI), lu en temps réel. */
  | { kind: "media_file"; path: string; loop?: boolean }
  /** Flux SRT entrant (`srt://hôte:port`, `?mode=listener` pour attendre l'émetteur). */
  | { kind: "srt_input"; url: string };

export type SourceKindName = SourceKind["kind"];

/** Tous les types de source, dans l'ordre de l'énumération Rust. */
export const SOURCE_KINDS: readonly SourceKindName[] = Object.freeze([
  "color",
  "image",
  "test_pattern",
  "camera",
  "display_capture",
  "window_capture",
  "audio_input",
  "media_file",
  "srt_input",
]);

/** Sources alimentées par une capture du moteur (miroir de `SourceKind::is_capture`). */
export const CAPTURE_SOURCE_KINDS: ReadonlySet<SourceKindName> = new Set<SourceKindName>([
  "camera",
  "display_capture",
  "window_capture",
  "audio_input",
  "media_file",
  "srt_input",
]);

/** Sources qui peuvent produire du son (miroir de `SourceKind::has_audio`). */
export const AUDIO_SOURCE_KINDS: ReadonlySet<SourceKindName> = new Set<SourceKindName>([
  "audio_input",
  "media_file",
  "srt_input",
]);

/** Vrai si la source passe par un pipeline de capture du moteur. */
export function isCaptureKind(kind: SourceKindName): boolean {
  return CAPTURE_SOURCE_KINDS.has(kind);
}

/** Vrai si la source peut avoir du son (elle a alors une tranche dans le mixeur). */
export function hasAudio(kind: SourceKindName): boolean {
  return AUDIO_SOURCE_KINDS.has(kind);
}

/** Volume maximal d'une source (gain linéaire, environ +6 dB). */
export const MAX_VOLUME = 2;

export type Source = {
  id: Uuid;
  name: string;
  transform: Required<Transform>;
  visible: boolean;
  /**
   * Gain audio linéaire de 0 à {@link MAX_VOLUME} (1 = inchangé). Toujours envoyé par
   * le moteur actuel ; optionnel pour les états construits à la main.
   */
  volume?: number;
  /** Son coupé (faux par défaut). */
  muted?: boolean;
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

/** Backend des sorties : GStreamer in-process ou ffmpeg externe (développement). */
export type OutputBackend = "gstreamer" | "ffmpeg";

/** Conteneur d'enregistrement. */
export type RecordFormat = "mp4" | "mkv";

/** Description d'une sortie active (réponse de Start*, `OutputStarted`). */
export interface OutputStartedData {
  output: OutputKind;
  target: string;
  /** Absents avec un moteur antérieur au backend GStreamer. */
  backend?: OutputBackend;
  /** Élément d'encodage H.264 utilisé (`vtenc_h264_hw`, `openh264enc`…). */
  encoder?: string;
  bitrate_kbps?: number;
  /** Enregistrement seulement. */
  format?: RecordFormat;
}

export interface OutputState extends Partial<Omit<OutputStartedData, "output">> {
  active: boolean;
  /** Pipeline en construction : la sortie n'est pas encore active. */
  starting?: boolean;
  frames?: number;
  dropped_frames?: number;
  duration_s?: number;
}

export interface OutputStatus {
  backend?: OutputBackend;
  record: OutputState;
  stream: OutputState;
}

export interface OutputStoppedData extends OutputStartedData {
  frames: number;
  dropped_frames?: number;
  duration_s: number;
  /** Vrai si le fichier a été finalisé proprement. */
  finalized?: boolean;
  warning?: string;
}

/** Options de `StartRecording` (toutes optionnelles). */
export interface StartRecordingData {
  path?: string;
  /** `mp4` par défaut ; déduit de l'extension de `path` si absent. */
  format?: RecordFormat;
  /** MP4 fragmenté, lisible même après un arrêt brutal. */
  fragmented?: boolean;
  /** Débit vidéo en kbit/s (100 à 100000). */
  bitrate_kbps?: number;
}

export interface StartStreamData {
  url: string;
  /** Débit vidéo en kbit/s (100 à 100000). */
  bitrate_kbps?: number;
}

/** Un encodeur H.264 connu du moteur (`GetEncoders`). */
export interface EncoderInfo {
  name: string;
  hardware: boolean;
  available: boolean;
  /** Celui qu'utilisera la prochaine sortie. */
  selected: boolean;
  /** Sous GPL (`x264enc`) : jamais choisi automatiquement. */
  gpl: boolean;
}

export interface EncodersInfo {
  backend: OutputBackend;
  /** `auto` ou nom demandé au lancement de studiod. */
  requested: string;
  encoders: EncoderInfo[];
}

/** Un périphérique d'entrée (`GetInputDevices`). */
export interface InputDevice {
  /** À passer en `device_id`. */
  id: string;
  name: string;
  /** Résumé lisible des formats. */
  caps_summary: string;
}

export interface InputDevices {
  video: InputDevice[];
  audio: InputDevice[];
}

/** Types de source qui émettent `SourceStarted` / `SourceFailed`. */
export type CaptureKindName = Exclude<SourceKindName, "color" | "image" | "test_pattern">;

/** État d'une capture (`GetCaptureStatus`). */
export type CaptureState =
  | { source_id: Uuid; kind: CaptureKindName; state: "starting" | "running" }
  | { source_id: Uuid; kind: CaptureKindName; state: "failed"; error: string; retry_in_ms: number };

export interface SourceStartedData {
  source_id: Uuid;
  kind: CaptureKindName;
}

export interface SourceFailedData {
  source_id: Uuid;
  kind: CaptureKindName;
  error: string;
  /** Délai avant la relance automatique (1 s à 30 s). */
  retry_in_ms: number;
}

/** Niveaux d'une source sur les 100 dernières ms, après le volume. */
export interface SourceLevels {
  source_id: Uuid;
  /** Crête par canal (gauche, droite), en dBFS ; -100 pour le silence. */
  peak_db: number[];
  /** RMS par canal, en dBFS ; -100 pour le silence. */
  rms_db: number[];
  muted: boolean;
  /** La source part dans le mix (scène programme, visible, non coupée). */
  audible: boolean;
}

export interface InputLevelsData {
  sources: SourceLevels[];
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
  /** Gain linéaire de 0 à 2. */
  volume?: number;
  muted?: boolean;
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
  GetCaptureStatus: RequestSpec<void, { sources: CaptureState[] }>;
  // studio.preview
  GetScreenshot: RequestSpec<{ scene?: string; format?: "png" | "jpeg" } | void, { image: string }>;
  // studio.outputs
  StartRecording: RequestSpec<StartRecordingData | void, OutputStartedData>;
  StopRecording: RequestSpec<void, OutputStoppedData>;
  StartStream: RequestSpec<StartStreamData, OutputStartedData>;
  StopStream: RequestSpec<void, OutputStoppedData>;
  GetOutputStatus: RequestSpec<void, OutputStatus>;
  // studio.encoders
  GetEncoders: RequestSpec<void, EncodersInfo>;
  // studio.inputs
  GetInputDevices: RequestSpec<void, InputDevices>;
  // studio.audio
  SetSourceVolume: RequestSpec<
    { scene: string; source: string; volume: number },
    { scene_id: Uuid; source_id: Uuid; volume: number }
  >;
  SetSourceMuted: RequestSpec<
    { scene: string; source: string; muted: boolean },
    { scene_id: Uuid; source_id: Uuid; muted: boolean }
  >;
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
  SourceStarted: SourceStartedData;
  SourceFailed: SourceFailedData;
  SourceVolumeChanged: { scene_id: Uuid; source_id: Uuid; volume: number };
  SourceMuteChanged: { scene_id: Uuid; source_id: Uuid; muted: boolean };
  InputLevels: InputLevelsData;
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
  GetCaptureStatus: "studio.sources",
  GetScreenshot: "studio.preview",
  StartRecording: "studio.outputs",
  StopRecording: "studio.outputs",
  StartStream: "studio.outputs",
  StopStream: "studio.outputs",
  GetOutputStatus: "studio.outputs",
  GetEncoders: "studio.encoders",
  GetInputDevices: "studio.inputs",
  SetSourceVolume: "studio.audio",
  SetSourceMuted: "studio.audio",
});

/** Domaines annoncés par le moteur (miroir de `CAPABILITIES` dans api.rs). */
export const STUDIO_CAPABILITIES: readonly string[] = Object.freeze([
  "core",
  "studio.scenes",
  "studio.sources",
  "studio.outputs",
  "studio.encoders",
  "studio.preview",
  "studio.inputs",
  "studio.audio",
]);

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
  "SourceStarted",
  "SourceFailed",
  "SourceVolumeChanged",
  "SourceMuteChanged",
  "InputLevels",
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
  UnsupportedUrl: "unsupported_url",
  InvalidBitrate: "invalid_bitrate",
  ShuttingDown: "shutting_down",
  InvalidVolume: "invalid_volume",
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
    case "SourceVolumeChanged": {
      const { scene_id, source_id, volume } = data as StudioEvents["SourceVolumeChanged"];
      return mapSource(state, scene_id, source_id, (x) => (x.volume === volume ? x : { ...x, volume }));
    }
    case "SourceMuteChanged": {
      const { scene_id, source_id, muted } = data as StudioEvents["SourceMuteChanged"];
      return mapSource(state, scene_id, source_id, (x) => (x.muted === muted ? x : { ...x, muted }));
    }
    default:
      return state;
  }
}

/** Applique `f` à une source ; renvoie le même état si rien ne change. */
function mapSource(state: StudioState, sceneId: Uuid, sourceId: Uuid, f: (source: Source) => Source): StudioState {
  const scene = state.scenes.find((s) => s.id === sceneId);
  const source = scene?.sources.find((x) => x.id === sourceId);
  if (!source) return state;
  const next = f(source);
  if (next === source) return state;
  return mapScene(state, sceneId, (s) => ({ ...s, sources: s.sources.map((x) => (x.id === sourceId ? next : x)) }));
}

/** Recherche pratique d'une scène par identifiant. */
export function findScene(state: StudioState, sceneId: Uuid): Scene | undefined {
  return state.scenes.find((s) => s.id === sceneId);
}
