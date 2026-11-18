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
serveur → {"op":"hello","app":{"id":"studio","name":"Maya Studio","version":"0.1.0"},
           "protocol":1,"auth_required":true,
           "capabilities":["core","studio.scenes","studio.sources","studio.outputs","studio.encoders","studio.preview",
                           "studio.inputs","studio.audio"]}

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

# Domaines de Maya Studio (`app.id = "studio"`)

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
| `UpdateSource` | `scene`, `source`, puis au choix `name`, `visible`, `transform`, `volume`, `muted`, `kind` + réglages |
| `RemoveSource` | `scene`, `source` |
| `MoveSource` | `scene`, `source`, `index` (0 = tout en dessous) |
| `GetCaptureStatus` | — (état des captures en cours, voir plus bas) |

Types de sources (`kind`) et leurs réglages :

| `kind` | Réglages | Image | Son |
|---|---|---|---|
| `color` | `rgba` | ✓ | |
| `image` | `path` (PNG ou JPEG) | ✓ | |
| `test_pattern` | — | ✓ | |
| `camera` | `device_id`, `width`, `height`, `fps` (tous optionnels) | ✓ | |
| `display_capture` | `display_id` optionnel (index d'écran ; sous X11 numéro d'écran ou nom d'affichage) | ✓ | |
| `window_capture` | `window_id` (XID sous X11, HWND sous Windows ; non disponible sous macOS) | ✓ | |
| `audio_input` | `device_id` optionnel | | ✓ |
| `media_file` | `path` (chemin local ou URI), `loop` (faux par défaut) | ✓ | ✓ |
| `srt_input` | `url` (`srt://hôte:port`, `?mode=listener` pour attendre l'émetteur) | ✓ | ✓ |

À venir : `ndi_input`, `browser`.

- `device_id` : identifiant renvoyé par `GetInputDevices`. Sans `device_id`, le
  périphérique par défaut de la plateforme. Un identifiant inconnu est essayé comme
  chemin ou index pour l'élément de la plateforme (par exemple `/dev/video2` sous Linux).
- `width` / `height` / `fps` d'une caméra : mode souhaité ; l'image est mise à l'échelle
  si la caméra ne le propose pas.
- Une image de capture est étirée dans le rectangle `transform` (sans conserver le
  rapport largeur / hauteur : c'est au client de choisir un rectangle adapté).
- Les sources de capture tournent dès qu'elles sont dans **une** scène, même hors
  antenne : passer une scène à l'antenne est instantané.
- `media_file` est lu en temps réel. Sans `loop`, la dernière image reste affichée à la fin.

Chaque source porte aussi `volume` (gain linéaire de 0 à 2, 1 par défaut) et `muted`
(faux par défaut), voir `studio.audio`.

`transform` : `x`, `y`, `width`, `height` en pixels du canvas, `opacity` de 0 à 1.

Événements : `SourceAdded`, `SourceUpdated`, `SourceRemoved`, `SourceReordered`, et pour
les sources de capture :

- `SourceStarted` : `source_id`, `kind` — la capture livre ses premières images ou son
  premier son (aussi après une reprise) ; une entrée SRT en écoute sans émetteur ne
  l'émet qu'à l'arrivée du flux ;
- `SourceFailed` : `source_id`, `kind`, `error`, `retry_in_ms` — la capture a échoué
  (périphérique absent ou débranché, fichier illisible, flux SRT coupé) ; elle est
  relancée automatiquement après `retry_in_ms` (de 1 s à 30 s, délai doublé à chaque
  échec). La source ne dessine rien pendant la panne.

`GetCaptureStatus` donne l'état courant de chaque capture, pour un client qui se connecte
après ces événements : une entrée par source de capture présente dans une scène,
`source_id`, `kind`, `state` (`starting` tant que rien n'est arrivé, `running`, `failed`) et,
pour `failed`, `error` et `retry_in_ms` du dernier échec.

```json
{"sources":[{"source_id":"…","kind":"media_file","state":"running"},
            {"source_id":"…","kind":"audio_input","state":"failed",
             "error":"pulsesrc0 : Failed to connect: Connection refused","retry_in_ms":4000}]}
```

### `studio.outputs`

| Requête | Données |
|---|---|
| `StartRecording` | `path` optionnel, `format` optionnel (`mp4` par défaut ou `mkv`, sinon déduit de l'extension de `path`), `fragmented` optionnel (MP4 fragmenté, lisible même après un arrêt brutal), `bitrate_kbps` optionnel (6000 par défaut) |
| `StopRecording` | — |
| `StartStream` | `url` (`rtmp://…/clé`, `rtmps://…/clé` ou `srt://hôte:port`), `bitrate_kbps` optionnel (4500 par défaut) |
| `StopStream` | — |
| `GetOutputStatus` | — |

Chaque sortie porte une piste vidéo H.264 et une piste audio AAC 48 kHz stéréo (le mix
des sources audio, voir `studio.audio`).
Conteneurs : MP4 ou Matroska pour l'enregistrement, FLV pour RTMP(S), MPEG-TS pour SRT.

Réponse de `StartRecording` / `StartStream` et données de `OutputStarted` :

```json
{"output":"record","target":"recordings/enregistrement-1794907353.mp4","backend":"gstreamer",
 "encoder":"openh264enc","bitrate_kbps":6000,"format":"mp4"}
```

`StopRecording` / `StopStream` et `OutputStopped` ajoutent `frames`, `dropped_frames`,
`duration_s`, `finalized` (vrai si le fichier a été finalisé proprement) et, en cas de
problème, `warning`.

`GetOutputStatus` :

```json
{"backend":"gstreamer",
 "record":{"active":true,"target":"…","encoder":"vtenc_h264_hw","bitrate_kbps":6000,"format":"mp4",
           "frames":912,"dropped_frames":0,"duration_s":30.4},
 "stream":{"active":false}}
```

`dropped_frames` compte les images perdues parce que l'encodeur ou le réseau ne suivait pas :
la boucle de rendu n'attend jamais une sortie. Une sortie en cours de démarrage (pipeline
en construction) apparaît avec `"active":false,"starting":true`.

Codes d'erreur : `output_already_active` (sortie active ou en démarrage), `output_not_active`,
`missing_url`, `unsupported_url`, `invalid_bitrate`, `shutting_down` (studiod est en train de
s'arrêter), `output_error` (détail dans `message`, par exemple encodeur introuvable).

Événements : `OutputStarted`, `OutputStopped`, `OutputFailed` (`output`, `error` : la sortie
s'est arrêtée seule, par exemple connexion RTMP refusée, connexion SRT refusée ou perdue,
disque plein). En SRT, la reconnexion automatique de `srtsink` est désactivée : une
connexion perdue arrête la sortie avec `OutputFailed` au lieu de la laisser « active » sans
rien envoyer ; c'est au client de relancer `StartStream`.

`SetCanvas` est refusé (`outputs_active`) tant qu'une sortie est active ou en démarrage.

À la réception de SIGINT (Ctrl-C) ou SIGTERM, `studiod` arrête proprement toutes les sorties
(EOS, finalisation des MP4) avant de quitter.

### `studio.encoders`

| Requête | Données | Réponse |
|---|---|---|
| `GetEncoders` | — | `backend`, `requested` (`auto` ou nom demandé au lancement), `encoders` |

Chaque entrée de `encoders` : `name` (élément GStreamer), `hardware`, `available`,
`selected` (celui qu'utilisera la prochaine sortie), `gpl` (vrai pour `x264enc`, jamais
choisi automatiquement).

```json
{"backend":"gstreamer","requested":"auto","encoders":[
  {"name":"nvh264enc","hardware":true,"available":false,"selected":false,"gpl":false},
  {"name":"vah264enc","hardware":true,"available":true,"selected":true,"gpl":false},
  {"name":"openh264enc","hardware":false,"available":true,"selected":false,"gpl":false},
  {"name":"x264enc","hardware":false,"available":true,"selected":false,"gpl":true}]}
```

Avec le backend `ffmpeg`, la liste est vide.

### `studio.inputs`

| Requête | Données | Réponse |
|---|---|---|
| `GetInputDevices` | — | `video`, `audio` : périphériques d'entrée présents |

Chaque entrée : `id` (à passer en `device_id`), `name`, `caps_summary` (résumé lisible des
formats). Les listes peuvent être vides (aucun périphérique, ou pas de fournisseur de
périphériques sur la machine) : ce n'est pas une erreur.

```json
{"video":[{"id":"/dev/video0","name":"Integrated Camera",
           "caps_summary":"video/x-raw (YUY2), image/jpeg · jusqu'à 1920x1080"}],
 "audio":[{"id":"alsa_input.usb-Focusrite-00","name":"Scarlett 2i2",
           "caps_summary":"audio/x-raw (S32LE) · 2 canaux · 48000 Hz"}]}
```

### `studio.audio`

| Requête | Données |
|---|---|
| `SetSourceVolume` | `scene`, `source`, `volume` (gain linéaire de 0 à 2 ; 1 = inchangé, 2 ≈ +6 dB) |
| `SetSourceMuted` | `scene`, `source`, `muted` |

Le mixeur somme, par blocs de 10 ms, les sources qui ont du son (`audio_input`,
`media_file`, `srt_input`) de la **scène programme**, visibles et non coupées, chacune avec
son volume, puis écrête le résultat. Ce mix part dans la piste AAC de chaque sortie active
(du silence s'il n'y a aucune source). L'audio est horodaté sur la même horloge que la
vidéo de la sortie : chaque bloc est placé par rapport à la position de la dernière image
envoyée, ce qui garde les deux pistes calées même si le rendu prend du retard.

Codes d'erreur : `invalid_volume` (hors de 0..2), `scene_not_found`, `source_not_found`.

Événements :

- `SourceVolumeChanged` : `scene_id`, `source_id`, `volume` ;
- `SourceMuteChanged` : `scene_id`, `source_id`, `muted` ;
- `InputLevels` (environ 10 fois par seconde, seulement s'il y a des sources audio) :
  niveaux sur les 100 dernières ms, après le volume, en dBFS par canal (gauche, droite),
  arrondis au dixième, plancher à -100 pour le silence. `muted` et `audible` (la source
  part dans le mix) permettent d'afficher un VU-mètre grisé.

```json
{"sources":[{"source_id":"…","peak_db":[-6.1,-6.0],"rms_db":[-9.1,-9.0],
             "muted":false,"audible":true}]}
```

Un client qui n'affiche pas de VU-mètres peut ignorer `InputLevels` (ou s'identifier avec
`events: false` s'il n'a besoin d'aucun événement).

### `studio.preview`

| Requête | Données |
|---|---|
| `GetScreenshot` | `scene` optionnelle (programme par défaut), `format` `png` ou `jpeg` |

Réponse : `image` en data URL.
