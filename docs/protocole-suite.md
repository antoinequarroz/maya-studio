# Protocole Suite Media — v1

Ce protocole est le **contrat commun à toutes les apps de la Suite Media** : le Studio,
Nino (régie automatique), les futures apps mobiles de capture et le studio web.
Chaque app expose un serveur WebSocket qui parle exactement ce protocole ; une app
qui veut en piloter une autre s'y connecte comme client.

Ce qui est commun à toutes les apps :

- l'enveloppe des messages (`op`) ;
- la poignée de main (`hello` → `identify` → `identified`) ;
- l'authentification par jeton ;
- le format des réponses et des codes d'erreur ;
- le format des événements ;
- le domaine de requêtes `core` ;
- la découverte (ports par défaut, mDNS).

Ce qui est propre à chaque app : ses domaines de requêtes (`studio.*`, `nino.*`…) et ses événements.

Le code de référence est le crate Rust `crates/suite-protocol`. Toute app de la suite
(Rust, TypeScript, Swift, Kotlin) doit produire et accepter les mêmes JSON.

## Transport

- WebSocket, messages **texte** contenant un objet JSON.
- Encodage UTF-8, noms de champs en `snake_case`, noms de requêtes et d'événements en `PascalCase`.
- Identifiants (scènes, sources…) : UUID v4 en texte. Partout où une requête attend
  une scène ou une source, on peut aussi passer son **nom**.

## Poignée de main

```
serveur → {"op":"hello","app":{"id":"studio","name":"Studio","version":"0.1.0"},
           "protocol":1,"auth_required":true,
           "capabilities":["core","studio.scenes","studio.sources","studio.outputs","studio.preview"]}

client  → {"op":"identify","token":"…","name":"Nino plateau 1",
           "app":{"id":"nino","name":"Nino","version":"0.3.0"},"events":true}

serveur → {"op":"identified","client_id":"1"}
```

- `protocol` : version majeure. Un client refuse un serveur dont la version majeure diffère.
- `capabilities` : domaines proposés. Un client teste un domaine avant de l'utiliser,
  ce qui permet à deux apps de versions différentes de rester compatibles.
- `events: false` : le client ne reçoit pas les événements (utile pour un script ponctuel).
- Toute requête envoyée avant `identified` reçoit l'erreur `not_identified`.

## Authentification

Un **jeton partagé** par installation de la suite. Le même jeton ouvre toutes les apps
d'un même poste ou d'un même plateau. Si le jeton est faux, le serveur répond
`auth_failed` puis ferme la connexion.

Évolution prévue (v1.x, rétrocompatible) : jetons par app avec droits par domaine
(par exemple Nino peut couper les scènes mais pas lancer la diffusion).

## Requêtes et réponses

```
client  → {"op":"request","id":"42","type":"SetProgramScene","data":{"scene":"Plateau"}}
serveur → {"op":"response","id":"42","ok":true,"data":{"scene_id":"…"}}
serveur → {"op":"response","id":"43","ok":false,
           "error":{"code":"scene_not_found","message":"scène introuvable : Plateu"}}
```

- `id` est choisi par le client et renvoyé tel quel. Les réponses peuvent arriver dans le désordre.
- `error.code` est **stable** et sert aux programmes ; `error.message` est pour les humains et peut changer.

Codes communs : `invalid_message`, `not_identified`, `auth_failed`, `unknown_request`, `invalid_data`.

## Événements

```
serveur → {"op":"event","app":"studio","type":"ProgramSceneChanged","data":{"scene_id":"…"}}
```

Chaque changement d'état est diffusé à tous les clients identifiés, **y compris celui qui l'a
demandé**. Un client peut donc se contenter d'écouter les événements pour tenir son état à jour.
`app` indique l'app émettrice, ce qui permet à un client branché sur plusieurs apps de tout
traiter dans une seule boucle.

## Domaine `core` (obligatoire pour toutes les apps)

| Requête | Données | Réponse |
|---|---|---|
| `GetAppInfo` | — | `app`, `protocol`, `capabilities` |
| `GetRequestList` | — | `requests` : liste des requêtes acceptées |
| `Ping` | — | `pong: true` |

## Découverte

- Ports par défaut : Studio `4460`, Nino `4461` (les nouvelles apps prennent les suivants).
- Sur le réseau local, chaque app s'annonce en mDNS / Bonjour sous `_suitemedia._tcp`
  avec les champs TXT `app`, `version`, `protocol` *(à implémenter, ticket dédié)*.

## Versionnement

- Ajouter une requête, un événement, un champ optionnel ou un domaine : **compatible**, pas de changement de version majeure.
- Renommer ou supprimer quelque chose, changer un type : **nouvelle version majeure** du protocole.
- Un client ignore les champs et événements qu'il ne connaît pas.

---

# Domaines du Studio (`app.id = "studio"`)

### `studio.scenes`

| Requête | Données |
|---|---|
| `GetStudio` | — (état complet : canvas, scènes, sources, programme, aperçu) |
| `GetSceneList` | — |
| `GetScene` | `scene` |
| `CreateScene` | `name` |
| `RemoveScene` | `scene` (refusé si à l'antenne : `scene_on_air`) |
| `RenameScene` | `scene`, `name` |
| `SetProgramScene` | `scene` (passe à l'antenne) |
| `SetPreviewScene` | `scene` |
| `Cut` | — (aperçu ↔ programme) |
| `SetCanvas` | `width`, `height`, `fps` (refusé si une sortie tourne) |

Événements : `SceneCreated`, `SceneRemoved`, `SceneRenamed`, `ProgramSceneChanged`,
`PreviewSceneChanged`, `CanvasChanged`.

### `studio.sources`

| Requête | Données |
|---|---|
| `AddSource` | `scene`, `name`, `kind` + réglages, `transform` optionnel |
| `UpdateSource` | `scene`, `source`, puis au choix `name`, `visible`, `transform`, `kind` + réglages |
| `RemoveSource` | `scene`, `source` |
| `MoveSource` | `scene`, `source`, `index` (0 = tout en dessous) |

Types de sources (`kind`) : `color` (`rgba`), `image` (`path`), `test_pattern`.
À venir avec GStreamer : `camera`, `display_capture`, `window_capture`, `audio_input`,
`media_file`, `srt_input`, `ndi_input`, `browser`.

`transform` : `x`, `y`, `width`, `height` en pixels du canvas, `opacity` de 0 à 1.

Événements : `SourceAdded`, `SourceUpdated`, `SourceRemoved`, `SourceReordered`.

### `studio.outputs`

| Requête | Données |
|---|---|
| `StartRecording` | `path` optionnel |
| `StopRecording` | — |
| `StartStream` | `url` (`rtmp://…/clé` ou `srt://…`) |
| `StopStream` | — |
| `GetOutputStatus` | — |

Événements : `OutputStarted`, `OutputStopped`, `OutputFailed`.

### `studio.preview`

| Requête | Données |
|---|---|
| `GetScreenshot` | `scene` optionnelle (programme par défaut), `format` `png` ou `jpeg` |

Réponse : `image` en data URL.
