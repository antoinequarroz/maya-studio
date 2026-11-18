# @suite-media/client

Client TypeScript du [protocole Suite Media v1](../../docs/protocole-suite.md), sans
dépendance. Il tourne partout où il y a un WebSocket standard : navigateur, Tauri,
Node 22+ (WebSocket global), React Native. Il sert à l'interface desktop de Maya Studio,
et servira à Nino, au futur studio web et aux apps mobiles.

```ts
import { StudioClient, isSuiteError } from "@suite-media/client";

const studio = new StudioClient({ url: "ws://127.0.0.1:4460", token: process.env.STUDIO_TOKEN, name: "Nino" });
await studio.connect();                       // hello → identify → identified

studio.subscribe("ProgramSceneChanged", ({ scene_id }) => console.log("à l'antenne :", scene_id));
await studio.call("CreateScene", { name: "Plateau" });   // requête typée
try {
  await studio.call("SetProgramScene", { scene: "Plateu" });
} catch (e) {
  if (isSuiteError(e, "scene_not_found")) console.warn(e.message);
}
```

- `SuiteClient` : client générique (n'importe quelle app de la suite) — `request<T>()`,
  `on(app, type, handler)` avec jokers `*`, `waitForEvent()`, `onState()`, `onError()`,
  `hasCapability()` / `requireCapability()`.
- `StudioClient` : requêtes (`call`) et événements (`subscribe`) de Maya Studio typés,
  vérification du domaine requis avant l'envoi (`capability_missing`).
- `applyStudioEvent(state, type, data)` : réducteur pur qui tient un état local à jour
  à partir des événements (après un `GetStudio` initial).
- Erreurs : `SuiteError` avec un `code` stable — code du serveur (`scene_not_found`,
  `auth_failed`…) ou du client (`timeout`, `not_connected`, `connection_closed`,
  `protocol_mismatch`, `capability_missing`, `aborted`).
- Reconnexion automatique avec backoff exponentiel et part aléatoire (`reconnect`),
  délais de connexion et de requête (`connectTimeoutMs`, `requestTimeoutMs`).
  `auth_failed` et une version de protocole différente sont fatals : pas de reconnexion.

Les types de `src/studio.ts` sont le miroir de `crates/studiod/src/api.rs` ; un test
vérifie que la liste des requêtes et des événements reste alignée avec le code Rust.

## Tests

```sh
npm test                    # unitaires + intégration
npm run test:unit
npm run test:integration    # lance le vrai studiod (cargo build -p studiod si besoin, ffmpeg requis)
```

Variables : `STUDIOD_BIN` (chemin du binaire), `CARGO_TARGET_DIR`.
