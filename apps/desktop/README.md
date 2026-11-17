# Maya Studio — interface desktop

Tauri 2 + React + Vite + TypeScript. Le shell Tauri lance le moteur `studiod` en
**sidecar** sur `127.0.0.1`, avec un port libre et un jeton aléatoires, et les donne
au front-end par la commande `studio_connection`. Le front-end ne parle au moteur
**que** par WebSocket, via [`@suite-media/client`](../../packages/suite-client) : le même
front-end tournera tel quel dans un navigateur face à un studio distant.

Cycle de vie du moteur lancé par l'app :

- il tourne dans son propre groupe de processus (objet job sous Windows) : l'arrêter
  emporte aussi ses processus enfants ;
- il reçoit `--exit-on-stdin-eof` : si l'app meurt brutalement (arrêt forcé, plantage,
  recompilation en `tauri dev`), le tube de son entrée standard se ferme et il s'arrête
  (sorties finalisées) au lieu de continuer à tourner en arrière-plan ;
- à la fermeture de l'app, il reçoit d'abord SIGTERM (macOS, Linux) et a jusqu'à 10 s pour
  finaliser les enregistrements et arrêter les captures, avant l'arrêt forcé ; sous
  Windows, l'arrêt reste forcé (un enregistrement MKV ou MP4 fragmenté reste lisible) ;
- s'il plante, le shell le relance avec le même port et le même jeton (trois fois par
  minute au plus) et prévient l'interface par l'événement Tauri `studio-engine`.

## Écrans

- **Mode studio** : moniteurs Aperçu (vert) et Programme (rouge) côte à côte, bouton **Couper**.
- **Scènes** : créer, renommer, supprimer ; clic = aperçu, double-clic = programme.
- **Sources** de la scène en aperçu : ajouter (couleur, image, mire, **caméra**, **écran**,
  **micro / entrée audio**, **fichier vidéo**, **flux SRT**), masquer, monter / descendre,
  supprimer. Caméras et entrées audio se choisissent dans la liste des périphériques du
  moteur (`GetInputDevices`, par exemple une Scarlett 2i2), ou « par défaut du système ».
  Chaque source de capture porte une pastille d'état : *démarrage*, *en marche* ou
  *en échec* (erreur en infobulle et dans les propriétés ; le moteur réessaie seul).
- **Propriétés** : nom, réglages du type (couleur, chemin, fichier et boucle, URL SRT,
  périphérique), état de la capture, position, taille, opacité.
- **Audio** : une tranche par source sonore (micro, fichier vidéo, SRT) de la scène **à
  l'antenne** — c'est elle qui part dans le mix — avec volume de 0 à 200 % (double-clic :
  100 %), coupure et VU-mètre stéréo (crête et RMS en dBFS, `InputLevels`, descente douce
  et crête maintenue). Les niveaux ne passent pas par l'état React : chaque VU-mètre se
  redessine en `requestAnimationFrame`.
- **Sorties** : démarrer / arrêter l'enregistrement (format **MP4** ou **MKV**) et le direct
  (serveur RTMP/SRT + clé), durée, images et encodeur utilisé (`vtenc_h264_hw`,
  `openh264enc`…, matériel ou logiciel) en temps réel ; au repos, l'encodeur prévu.
- **Barre d'état** : connexion, canvas, cadence.
- Raccourcis : <kbd>Espace</kbd> ou <kbd>Entrée</kbd> = couper, <kbd>1</kbd>…<kbd>9</kbd> = scène en aperçu.

L'état est tenu à jour **uniquement par les événements** du studio (`applyStudioEvent`),
après un `GetStudio` à chaque (re)connexion : une action faite par Nino ou un autre
client apparaît immédiatement.

Les moniteurs interrogent `GetScreenshot` en JPEG environ 5 fois par seconde. C'est
provisoire : un vrai flux d'aperçu à faible latence (WebRTC, MJPEG ou mémoire partagée
côté Tauri) fera l'objet d'un ticket dédié.

## Lancer

Prérequis communs : Rust stable, Node 22+, GStreamer (voir le [README principal](../../README.md#installer-gstreamer)).
À la racine du dépôt : `npm install` puis `npm run build` (compile `@suite-media/client`).

### macOS

```sh
xcode-select --install          # outils de compilation, une fois
npm install && npm run build    # à la racine du dépôt
cd apps/desktop
npm run tauri dev
```

### Autorisations macOS (caméra, micro, écran)

- **Caméra et micro** : au premier ajout d'une source caméra ou micro, macOS demande
  l'autorisation avec les textes de [`src-tauri/Info.plist`](src-tauri/Info.plist)
  (`NSCameraUsageDescription`, `NSMicrophoneUsageDescription`, fusionné par Tauri dans
  l'app). Un refus se corrige dans Réglages Système → Confidentialité et sécurité →
  Caméra / Microphone.
- **Écran** : la capture d'écran n'affiche pas de demande avec bouton ; il faut cocher Maya
  Studio dans Réglages Système → Confidentialité et sécurité → **Enregistrement de
  l'écran** (macOS 15 : « Enregistrement de l'écran et de l'audio système »), puis relancer
  l'app. Sans cette autorisation, la source Écran passe *en échec*.
- Le moteur `studiod` est lancé par l'app (sidecar) : macOS attribue ses accès à l'app
  qui l'a lancé, Maya Studio. Les autorisations se donnent donc à **Maya Studio**, pas à
  `studiod`. En revanche, `studiod` lancé seul depuis un terminal (ou `npm run tauri dev`,
  dont le binaire n'est pas encore une app empaquetée) hérite des autorisations du
  terminal (Terminal, iTerm, VS Code…) : c'est lui qu'il faut autoriser en développement.

### Windows

Prérequis Tauri : [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
(« Développement Desktop en C++ ») et WebView2 (déjà présent sur Windows 10/11 récents).

```powershell
npm install; npm run build      # à la racine du dépôt
cd apps\desktop
npm run tauri dev
```

### Linux

Paquets WebKitGTK 4.1, GTK 3, libsoup 3, librsvg (voir la documentation Tauri), puis comme ci-dessus.

`npm run tauri dev` compile d'abord `studiod` et le copie dans
`src-tauri/binaries/studiod-<triplet>` (`npm run sidecar`), puis lance Vite et la fenêtre.
`npm run tauri build` produit l'installateur avec le moteur en release.

Variables utiles :

- `MAYA_STUDIO_URL` (+ `MAYA_STUDIO_TOKEN`) : l'app se branche sur un studio déjà lancé, sans sidecar ;
- `MAYA_STUDIO_RECORDINGS` : dossier des enregistrements (par défaut `~/Vidéos/Maya Studio`).

### Sans Tauri (navigateur)

```sh
export STUDIO_TOKEN=$(openssl rand -hex 16)   # jeton aléatoire, lu aussi par studiod
cargo run --release -p studiod                  # à la racine
cd apps/desktop
VITE_STUDIO_URL=ws://127.0.0.1:4460 VITE_STUDIO_TOKEN="$STUDIO_TOKEN" npm run dev
```

Ou ouvrir `http://localhost:5173/?url=ws://127.0.0.1:4460&token=<votre jeton>`. Sans configuration,
un écran de connexion demande l'adresse et le jeton.

Les chemins (image, fichier vidéo) et les périphériques sont ceux de la **machine du
moteur** : le front-end n'ouvre pas de sélecteur de fichier, il envoie le chemin saisi.

## Structure

```
src/
  connection.ts           d'où viennent l'URL et le jeton (Tauri, ?url=, VITE_*, écran de connexion)
  studio/StudioContext    client unique, état tenu par les événements, sorties, notifications
  studio/useScreenshot    moniteurs (interrogation JPEG)
  components/             moniteurs, scènes, sources, propriétés, audio, sorties, fenêtres modales
  audio/meter.ts          échelle et balistique des VU-mètres (sans React)
  sourceKinds.ts          libellés et résumés des types de source
test/                     tests Vitest (jsdom) : champs, raccourcis, sélection, erreurs fatales,
                          VU-mètres, choix des périphériques, encodeur affiché
src-tauri/
  src/lib.rs              lancement et relance du sidecar, commande studio_connection, arrêt à la fermeture
  tauri.conf.json         externalBin = binaries/studiod
  Info.plist              textes des autorisations caméra / micro (macOS)
scripts/prepare-sidecar.mjs
```

Le crate `src-tauri` est hors de l'espace de travail Cargo racine (il a son propre
`[workspace]`) pour que `cargo build` du moteur n'ait pas besoin de WebKit.
