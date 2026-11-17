# Maya Studio

Le studio de production et de diffusion de la **Suite Media** : scènes, sources,
compositing, enregistrement et diffusion RTMP/SRT, entièrement pilotable par
WebSocket. Nino (régie automatique) et les autres apps de la suite s'y branchent
via le [protocole commun de la suite](docs/protocole-suite.md).

## Où on en est (v0.1, squelette)

Fonctionne déjà :

- modèle de scènes et de sources, mode studio (aperçu / programme, coupe) ;
- compositeur logiciel de référence (couleur, image, mire animée, images de capture mises à
  l'échelle, position, opacité) ;
- vraies sources par GStreamer : caméras, capture d'écran et de fenêtre, entrées audio,
  fichiers média (en boucle ou non), SRT entrant, avec découverte des périphériques
  (`GetInputDevices`) et relance automatique d'une capture en échec ;
- mixeur audio (volume, mute, écrêtage) poussé dans les sorties, calé sur la vidéo, et
  VU-mètres (`InputLevels`, crête et RMS en dBFS) ;
- rendu programme en continu à la cadence du canvas (30 i/s en 1280×720 en build release) ;
- enregistrement MP4 / MKV et diffusion RTMP(S) / SRT en H.264 + AAC, avec un backend
  GStreamer in-process (LGPL) qui choisit l'encodeur matériel disponible
  (VideoToolbox, NVENC, QuickSync, AMF, Media Foundation, VA-API) et se replie sur openh264 ;
- capture d'écran PNG/JPEG de n'importe quelle scène ;
- API WebSocket complète selon le protocole de la suite, avec jeton et événements ;
- client TypeScript de la suite (`@suite-media/client`) et interface desktop Tauri + React
  (mode studio, scènes, sources de capture avec leur état, propriétés, mixeur audio avec
  VU-mètres, enregistrement MP4 / MKV et direct).

Prochaines étapes (voir le projet Linear « Media · Maya Studio ») :

1. Sources NDI et navigateur ; capture de fenêtre sous macOS (ScreenCaptureKit).
2. Audio : filtres (gain, noise gate, compresseur), moniteur casque, compensation de dérive
   d'horloge des interfaces audio.
3. Compositeur GPU wgpu (DirectX 12 / Metal / Vulkan).
4. Interface desktop : flux d'aperçu à faible latence (les moniteurs interrogent des
   captures JPEG pour l'instant).
5. Découverte mDNS des apps de la suite.

## Architecture

```
crates/
  suite-protocol/   contrat commun de la Suite Media (enveloppe, poignée de main, erreurs)
  studio-core/      modèle de scènes, opérations, compositeur
  studiod/          moteur sans interface : boucle de rendu, sorties, serveur WebSocket
    src/outputs/    trait Output, backend GStreamer (gst.rs) et backend ffmpeg provisoire
    src/encoders.rs choix des encodeurs H.264 / AAC à l'exécution
    src/captures.rs pipelines de capture (caméra, écran, fenêtre, micro, fichier, SRT)
    src/devices.rs  découverte des périphériques (gst::DeviceMonitor)
    src/media.rs    dernières images et files audio des sources (sans GStreamer)
    src/mixer.rs    calculs du mixeur audio (somme, volume, écrêtage, dBFS, calage A/V)
packages/
  suite-client/     client TypeScript du protocole (navigateur, Node, mobile), sans dépendance
apps/
  desktop/          interface Tauri 2 + React ; lance studiod en sidecar, parle en WebSocket
tools/
  demo-client.mjs   client de démo (fait ce que Nino fera)
docs/
  protocole-suite.md
```

L'interface ne parle au moteur **que** par l'API WebSocket, comme Nino : le même front-end
servira au futur studio web face à un moteur distant.

## Installer GStreamer

Le backend de sortie par défaut utilise GStreamer 1.20 ou plus récent (bibliothèques
de développement pour compiler, plugins pour encoder).

**macOS** (Homebrew) :

```sh
brew install gstreamer
# le paquet inclut base, good, bad (openh264, vtenc, srt, rtmp2), ugly et libav
```

Le GStreamer de Homebrew ne contient pas `openh264enc` : sur Mac, l'encodage passe donc
toujours par VideoToolbox (`vtenc_h264_hw`, ou `vtenc_h264` sans puce dédiée), et les tests
utilisent `vtenc_h264` à la place d'openh264.

VideoToolbox (`vtenc_h264_hw`) est fourni par gst-plugins-bad et utilisé automatiquement
sur les Mac Apple Silicon et Intel.

**Windows** : installer les **deux** paquets MSVC 64 bits officiels depuis
<https://gstreamer.freedesktop.org/download/> (« runtime » et « development »),
installation complète. Puis, dans un nouveau terminal :

```powershell
$env:PKG_CONFIG_PATH = "C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig"
$env:Path += ";C:\gstreamer\1.0\msvc_x86_64\bin"
```

(il faut aussi un `pkg-config`, par exemple `choco install pkgconfiglite`). Les encodeurs
NVENC, QuickSync, AMF et Media Foundation sont dans gst-plugins-bad et utilisés s'il y a le
matériel et le pilote.

**Linux** (Debian / Ubuntu) :

```sh
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav
# VA-API (Intel / AMD) : gstreamer1.0-vaapi ou les éléments va de plugins-bad
```

Pour compiler sans GStreamer : `cargo build -p studiod --no-default-features`
(seul le backend ffmpeg reste disponible).

## Lancer

Prérequis : Rust stable, GStreamer (voir ci-dessus), Node 22+ pour le client de démo.

```sh
export STUDIO_TOKEN=$(openssl rand -hex 16)   # jeton aléatoire
cargo run --release -p studiod
# dans un autre terminal
node tools/demo-client.mjs ws://127.0.0.1:4460 "$STUDIO_TOKEN"
# enregistrement MKV et diffusion SRT en même temps :
node tools/demo-client.mjs ws://127.0.0.1:4460 "$STUDIO_TOKEN" mkv "srt://127.0.0.1:9000"
# avec un fichier vidéo en incrustation (et son son dans l'enregistrement) :
node tools/demo-client.mjs ws://127.0.0.1:4460 "$STUDIO_TOKEN" --media ~/Vidéos/clip.mp4
```

Options de `studiod` : `--listen 127.0.0.1:4460`, `--token` (ou `STUDIO_TOKEN`),
`--recordings recordings`, `--width 1280 --height 720 --fps 30`,
`--output-backend gstreamer|ffmpeg`, `--encoder auto|<élément>`,
`--record-bitrate 6000`, `--stream-bitrate 4500` (kbit/s), `--exit-on-stdin-eof` (sidecar :
s'arrêter proprement quand l'app parente ferme l'entrée standard). Sur SIGINT / SIGTERM ou
fermeture de l'entrée standard, les sorties sont finalisées et les captures arrêtées
avant de quitter.

### Choisir l'encodeur

Par défaut (`--encoder auto`), `studiod` essaie dans l'ordre les encodeurs matériels de la
plateforme, puis le repli logiciel `openh264enc` :

| Plateforme | Ordre |
|---|---|
| macOS | `vtenc_h264_hw`, `vtenc_h264`, `openh264enc` |
| Windows | `nvh264enc`, `qsvh264enc`, `amfh264enc`, `mfh264enc`, `openh264enc` |
| Linux | `nvh264enc`, `vah264enc`, `vaapih264enc`, `openh264enc` |

Un encodeur est retenu s'il existe et s'initialise. L'encodeur choisi est écrit au
démarrage, renvoyé par `GetEncoders`, `OutputStarted` et `GetOutputStatus`. Pour en forcer
un : `--encoder nvh264enc`. `gst-inspect-1.0 | grep 264` liste ceux installés.

Réglages appliqués quand l'élément les propose : débit constant au débit demandé, image clé
toutes les 2 s, pas d'images B, mode faible latence.

`x264enc` est sous GPL : il n'est **jamais** choisi automatiquement, seulement avec
`--encoder x264enc`, et un avertissement est écrit dans le journal.

### Sorties

- Enregistrement : MP4 par défaut (`format: "mkv"` pour Matroska, `fragmented: true` pour un
  MP4 fragmenté qui reste lisible après un plantage). À l'arrêt, `studiod` envoie la fin de
  flux et attend jusqu'à 5 s que le fichier soit finalisé.
- Diffusion : `rtmp://` et `rtmps://` (FLV, `rtmp2sink`), `srt://` (MPEG-TS, `srtsink`).
- Audio : piste AAC 48 kHz stéréo (le mix des sources audio, du silence s'il n'y en a pas),
  encodeur `avenc_aac`, sinon `voaacenc`, sinon `fdkaacenc`.

### Sources de capture

Chaque source de capture présente dans une scène a son propre pipeline GStreamer, lancé à
l'ajout et arrêté quand la source n'est plus dans aucune scène. Les éléments sont choisis à
l'exécution selon la plateforme (le même binaire fonctionne avec ou sans les plugins) :

| Source | macOS | Windows | Linux |
|---|---|---|---|
| `camera` | `avfvideosrc` | `mfvideosrc`, `ksvideosrc` | `v4l2src` |
| `display_capture` | `avfvideosrc capture-screen=true` | `d3d11screencapturesrc` | `ximagesrc` (X11, XWayland) |
| `window_capture` | — | `d3d11screencapturesrc window-handle` | `ximagesrc xid` |
| `audio_input` | `osxaudiosrc` | `wasapi2src`, `wasapisrc` | `pulsesrc`, `pipewiresrc`, `alsasrc` |
| `media_file` | `uridecodebin` | `uridecodebin` | `uridecodebin` |
| `srt_input` | `srtsrc auto-reconnect=false ! decodebin` | idem | idem |

Avec un `device_id` issu de `GetInputDevices`, l'élément est créé par le périphérique
lui-même (`gst::Device::create_element`). Une capture qui échoue (caméra débranchée, pas
d'accès à l'écran, flux SRT coupé) émet `SourceFailed` et est relancée de 1 s à 30 s plus
tard. Sous macOS, la caméra, le micro et l'écran demandent l'autorisation de l'utilisateur
(Réglages Système → Confidentialité et sécurité ; voir
[apps/desktop/README.md](apps/desktop/README.md#autorisations-macos-caméra-micro-écran)). En session Wayland pure (sans `DISPLAY`), la capture
d'écran n'est pas encore prise en charge : elle demande le portail xdg-desktop-portal
(ScreenCast) et la source échoue avec un message clair plutôt que de capturer la webcam
par erreur ; une session X11 ou XWayland fonctionne avec `ximagesrc`.

`SourceStarted` est émis à la première image ou au premier son reçu (une entrée SRT en
écoute sans émetteur n'est pas « démarrée »). La reconnexion automatique de `srtsrc` est
désactivée : un flux coupé ou un hôte injoignable donne `SourceFailed`, l'image est
effacée et le superviseur relance la capture. Quand les réglages d'une capture changent,
le nouveau pipeline attend que l'ancien ait libéré le périphérique ou le port.

Dérive d'horloge audio : chaque entrée garde sa file proche de sa cible en dupliquant ou en
jetant une trame par bloc de 10 ms quand le remplissage minimal observé sur une seconde
sort de la plage 5–30 ms. Les écarts courants (10 à 100 ppm) sont ainsi rattrapés sans
trou de silence ni latence qui grimpe jusqu'à la borne de 200 ms.

Tester la diffusion SRT en local :

```sh
ffmpeg -i "srt://127.0.0.1:9000?mode=listener" -c copy sortie.ts
```

### Interface desktop (macOS, Windows, Linux)

Prérequis en plus : les outils de compilation de Tauri — Xcode Command Line Tools sur
macOS (`xcode-select --install`), Microsoft C++ Build Tools et WebView2 sur Windows,
WebKitGTK 4.1 sur Linux.

```sh
npm install          # espace de travail npm : packages/* et apps/*
npm run build        # compile @suite-media/client puis le front-end
cd apps/desktop
npm run tauri dev    # compile studiod, le lance en sidecar et ouvre la fenêtre
```

Sous Windows, même chose dans PowerShell (`cd apps\desktop`). `npm run tauri build`
produit l'installateur (`.dmg` / `.msi` / `.AppImage`). Détails, mode navigateur sans
Tauri (`VITE_STUDIO_URL`, `?url=…&token=…`) et variables : [apps/desktop/README.md](apps/desktop/README.md).

### Tests

```sh
cargo fmt --check && cargo clippy --all-targets && cargo test
npm test             # client TypeScript : unitaires + intégration contre le vrai studiod
```

## Licences

Le code de ce dépôt est propriétaire. On n'utilise **aucun code d'OBS** (GPL).
Les briques tierces doivent rester compatibles avec une distribution commerciale :
GStreamer (LGPL, chargé dynamiquement), crates Rust MIT/Apache, openh264 (BSD) comme
encodeur logiciel. Les encodeurs GPL (`x264enc`) ne sont utilisables qu'à la demande
explicite. Le backend `--output-backend ffmpeg` appelle un `ffmpeg` externe avec `libx264`
(GPL) : il sert au développement seulement.
