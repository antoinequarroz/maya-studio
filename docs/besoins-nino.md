# Ce dont Nino a besoin de Maya Studio

Ticket MAYA-73. Ce document liste ce que Nino (la régie automatique,
dépôt `nino-studio`) attend du studio pour remplacer OBS, et dit où en est le
protocole Suite Media pour chaque besoin.

Source : le pilote OBS de Nino (`studio/apps/api/src/obs.ts`), l'interface commune
des logiciels de capture (`capture.ts`), le pilote Maya déjà écrit (`maya.ts`), la
réalisation automatique (`auto-direction.ts`) et la production (`studio.ts`), tels
qu'ils sont sur la branche principale de `nino-studio` le 18 novembre 2026.

## Comment Nino pilote un logiciel de capture

Nino ne parle jamais directement à OBS depuis ses écrans : tout passe par un
« pilote » qui implémente la même interface pour OBS, Streamlabs et Maya Studio.
Trois règles comptent pour le studio :

1. **Aucune commande n'est supposée réussie.** Après chaque commande, Nino relit
   l'état (scène à l'antenne, enregistrement, diffusion) et ne croit que cette
   relecture. Il relit aussi l'état toutes les secondes.
2. **Une connexion perdue ne veut pas dire « arrêté ».** Nino garde le dernier état
   connu et affiche une erreur. Il ne rejoue jamais une commande.
3. **Une fonction absente est désactivée, jamais simulée.** Chaque pilote annonce
   ses capacités (`record.pause`, `audio.mute`, `audio.meters`,
   `scene.thumbnails`, `record.directory`, `stream.local`, `media.play`,
   `guests.browser`). Le bouton correspondant est grisé avec une explication.

Le pilote Maya actuel n'annonce que `record.directory`, `scene.thumbnails` et
`stream.local`. Le but est qu'il puisse annoncer les huit.

## Les besoins, un par un

| # | Besoin de Nino | Ce que Nino utilise dans OBS | Dans le protocole Maya | État |
|---|---|---|---|---|
| 1 | Se connecter, s'identifier, détecter une version incompatible | obs-websocket, mot de passe | `hello` / `identify`, jeton de la suite, `protocol` | ✅ v1 |
| 2 | Lire les scènes et celle à l'antenne | `GetSceneList` | `GetSceneList` | ✅ v1 |
| 3 | Passer une scène à l'antenne (coupe manuelle ou automatique) | `SetCurrentProgramScene` | `SetProgramScene` | ✅ v1 |
| 4 | Être prévenu des changements (scène, enregistrement, diffusion) | `CurrentProgramSceneChanged`, `RecordStateChanged`, `StreamStateChanged` | `ProgramSceneChanged`, `OutputStarted`, `OutputStopped`, `OutputFailed` | ✅ v1 |
| 5 | Miniatures des plans, petites et légères (320 × 180, JPEG) | `GetSourceScreenshot` avec taille et qualité | `GetScreenshot` + `width`, `height`, `quality` | ✅ v1.1 (ce ticket) |
| 6 | Démarrer et arrêter l'enregistrement d'une prise, récupérer le fichier | `StartRecord`, `StopRecord` (`outputPath`) | `StartRecording` (`path`), `StopRecording` (`target`) | ✅ v1 |
| 7 | Choisir le dossier d'enregistrement | `SetRecordDirectory` | Nino passe un `path` complet à chaque prise | ✅ v1 |
| 8 | Mettre l'enregistrement en pause et le reprendre | `PauseRecord`, `ResumeRecord`, `outputPaused` | `PauseRecording`, `ResumeRecording`, `paused` dans `GetOutputStatus`, événements `RecordingPaused` / `RecordingResumed` | ✅ v1.1 (ce ticket) |
| 9 | Durée et taille de l'enregistrement en cours (contrôle qu'il avance) | `outputTimecode`, `outputBytes` | `duration_s` (pauses exclues), `bytes` | ✅ v1.1 (ce ticket) |
| 10 | Envoyer le programme vers le relais local de la salle (aperçu, retour) | service RTMP personnalisé, `StartStream` | `StartStream` avec `url` | ✅ v1 |
| 11 | Savoir si la diffusion se reconnecte | `outputReconnecting` | `reconnecting` dans `GetOutputStatus` | 🔜 MAYA-115 (reconnexion RTMP) |
| 12 | Liste des micros, coupés ou non, avec leur volume, **indépendamment des scènes** | `GetInputList`, `GetInputMute`, `GetInputVolume` | Entrées audio globales : `GetAudioInputs` (domaine `studio.mixer`) | ✅ v1.1 (ce ticket) |
| 13 | Couper et rétablir un micro (y compris les invités) | `SetInputMute` | `SetAudioInputMuted` (et `SetSourceMuted` pour une source de scène) | ✅ v1.1 (ce ticket) |
| 14 | Niveaux de chaque micro environ 10 fois par seconde, même hors antenne, pour la réalisation automatique | événement `InputVolumeMeters` | `InputLevels`, nouveau tableau `inputs` (clé `input_id`) | ✅ v1.1 (ce ticket) |
| 15 | Reconnaître ses propres sources (médias, invités) et ne jamais toucher celles de l'opérateur | réglages `nino_owner`, `nino_project`… cachés dans les réglages OBS | `meta` : objet JSON libre sur chaque source et entrée audio | ✅ v1.1 (ce ticket) |
| 16 | Jouer un média (vidéo, son, image) en plein écran ou en incrustation, le relancer, le mettre en pause, l'arrêter, savoir s'il est fini | `CreateInput` (`ffmpeg_source`), `SetSceneItemTransform`, `TriggerMediaInputAction`, `GetMediaInputStatus` | Scène + source `media_file` existent ; manquent `MediaAction` (relancer, pause, reprise, arrêt), `GetMediaStatus` (état, position, durée) et l'événement `MediaEnded` | 🔜 à faire (ticket à créer, phase 1) |
| 17 | Montrer et cacher une source dans une scène | `SetSceneItemEnabled` | `UpdateSource` avec `visible` | ✅ v1 |
| 18 | Placer une source (plein écran, incrustation en bas à droite) | `SetSceneItemTransform` | `UpdateSource` avec `transform` | ✅ v1 |
| 19 | Invités à distance : une page web par invité dans les scènes, son compris | source `browser_source`, `reroute_audio` | type de source `browser` (prévu dans la v1) | 🔜 à faire (ticket à créer) |
| 20 | Retour vidéo vers les invités | caméra virtuelle (`StartVirtualCam`) | sortie « caméra virtuelle » ou flux local (SRT, WebRTC) | 🔜 à faire (ticket à créer) |
| 21 | Le son des invités ne doit pas partir dans le retour casque | `SetInputAudioMonitorType` | retour casque (MAYA-101), avec un réglage par source | 🔜 avec MAYA-101 |
| 22 | Trouver le studio sur le réseau local sans saisir d'adresse | (aucun, adresse saisie) | mDNS `_suitemedia._tcp` | 🔜 MAYA-102 |

## Pourquoi des entrées audio globales

Dans OBS, une source audio est globale : le même micro peut être placé dans
plusieurs scènes, et Nino le désigne par son nom. La réalisation automatique
compare les niveaux des micros de chaque participant pour choisir le plan : elle a
besoin des niveaux de **tous** les micros en permanence, pas seulement de ceux de
la scène à l'antenne.

Dans Maya Studio v1, une source appartient à une scène. Mettre le même micro dans
trois scènes ouvre trois captures du même périphérique, et ses niveaux arrivent
sous trois identifiants différents.

Les **entrées audio globales** (domaine `studio.mixer`) règlent cela : un micro y est
déclaré une fois, avec un nom unique. Il part toujours dans le mix, quelle que soit
la scène à l'antenne, sauf s'il est coupé. C'est le fonctionnement d'une console
de mixage, et celui que Nino attend. Les sources audio des scènes restent
disponibles pour le son qui doit suivre l'image (un clip, un flux SRT).

## Ce que le pilote Maya de Nino pourra faire avec la v1.1

Le pilote client est hors de ce ticket (epic E0.2). Pour référence :

- `audio.mute` et `audio.meters` : remplir `inputs` avec `GetAudioInputs`, couper avec
  `SetAudioInputMuted`, convertir `InputLevels.inputs` en `AudioFrame` (le maximum
  des deux canaux, comme pour OBS).
- `record.pause` : `PauseRecording` / `ResumeRecording`, `paused` relu dans
  `GetOutputStatus`.
- `outputBytes` : `bytes` de l'enregistrement.
- Miniatures : `GetScreenshot` avec `width: 320`, `format: "jpeg"`, `quality: 65`.
- Avant d'utiliser une requête, le pilote vérifie qu'elle figure dans
  `GetRequestList` (ou que `studio.mixer` figure dans `capabilities`) : un studio
  plus ancien reste utilisable, la fonction est simplement grisée.

## Suite

1. Pilote Maya de Nino mis à jour pour la v1.1 (epic E0.2).
2. Lecture de médias pilotée (`MediaAction`, `GetMediaStatus`, `MediaEnded`).
3. Source `browser` et retour vidéo pour les invités.
4. État de reconnexion de la diffusion (MAYA-115), retour casque (MAYA-101),
   découverte mDNS (MAYA-102).
