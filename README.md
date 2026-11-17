# Maya Studio

Le studio de production et de diffusion de la **Suite Media** : scènes, sources,
compositing, enregistrement et diffusion RTMP/SRT, entièrement pilotable par
WebSocket. Nino (régie automatique) et les autres apps de la suite s'y branchent
via le [protocole commun de la suite](docs/protocole-suite.md).

## Où on en est (v0.1, squelette)

Fonctionne déjà :

- modèle de scènes et de sources, mode studio (aperçu / programme, coupe) ;
- compositeur logiciel de référence (couleur, image, mire animée, position, opacité) ;
- rendu programme en continu à la cadence du canvas (30 i/s en 1280×720 en build release) ;
- enregistrement MP4 / MKV et diffusion RTMP(S) / SRT en H.264 + AAC, avec un backend
  GStreamer in-process (LGPL) qui choisit l'encodeur matériel disponible
  (VideoToolbox, NVENC, QuickSync, AMF, Media Foundation, VA-API) et se replie sur openh264 ;
- capture d'écran PNG/JPEG de n'importe quelle scène ;
- API WebSocket complète selon le protocole de la suite, avec jeton et événements.

Prochaines étapes (voir le projet Linear « Media · Maya Studio ») :

1. Vraies sources : caméras, capture d'écran et de fenêtre, micros, fichiers média, SRT/NDI entrants.
2. Audio : mixage (poussé dans l'entrée audio des sorties, aujourd'hui du silence), VU-mètres.
3. Compositeur GPU wgpu (DirectX 12 / Metal / Vulkan).
4. Interface desktop Tauri + React (Windows, macOS).
5. Découverte mDNS des apps de la suite.

## Architecture

```
crates/
  suite-protocol/   contrat commun de la Suite Media (enveloppe, poignée de main, erreurs)
  studio-core/      modèle de scènes, opérations, compositeur
  studiod/          moteur sans interface : boucle de rendu, sorties, serveur WebSocket
    src/outputs/    trait Output, backend GStreamer (gst.rs) et backend ffmpeg provisoire
    src/encoders.rs choix des encodeurs H.264 / AAC à l'exécution
tools/
  demo-client.mjs   client de démo (fait ce que Nino fera)
docs/
  protocole-suite.md
```

## Installer GStreamer

Le backend de sortie par défaut utilise GStreamer 1.20 ou plus récent (bibliothèques
de développement pour compiler, plugins pour encoder).

**macOS** (Homebrew) :

```sh
brew install gstreamer
# le paquet inclut base, good, bad (openh264, vtenc, srt, rtmp2), ugly et libav
```

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
cargo run --release -p studiod -- --token monjeton
# dans un autre terminal
node tools/demo-client.mjs ws://127.0.0.1:4460 monjeton
# enregistrement MKV et diffusion SRT en même temps :
node tools/demo-client.mjs ws://127.0.0.1:4460 monjeton mkv "srt://127.0.0.1:9000"
```

Options de `studiod` : `--listen 127.0.0.1:4460`, `--token` (ou `STUDIO_TOKEN`),
`--recordings recordings`, `--width 1280 --height 720 --fps 30`,
`--output-backend gstreamer|ffmpeg`, `--encoder auto|<élément>`,
`--record-bitrate 6000`, `--stream-bitrate 4500` (kbit/s).

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
- Audio : piste AAC 48 kHz stéréo (silence pour l'instant), encodeur `avenc_aac`, sinon
  `voaacenc`, sinon `fdkaacenc`.

Tester la diffusion SRT en local :

```sh
ffmpeg -i "srt://127.0.0.1:9000?mode=listener" -c copy sortie.ts
```

## Licences

Le code de ce dépôt est propriétaire. On n'utilise **aucun code d'OBS** (GPL).
Les briques tierces doivent rester compatibles avec une distribution commerciale :
GStreamer (LGPL, chargé dynamiquement), crates Rust MIT/Apache, openh264 (BSD) comme
encodeur logiciel. Les encodeurs GPL (`x264enc`) ne sont utilisables qu'à la demande
explicite. Le backend `--output-backend ffmpeg` appelle un `ffmpeg` externe avec `libx264`
(GPL) : il sert au développement seulement.
