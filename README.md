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
- enregistrement MP4 H.264 et diffusion RTMP/SRT (backend provisoire : processus `ffmpeg`) ;
- capture d'écran PNG/JPEG de n'importe quelle scène ;
- API WebSocket complète selon le protocole de la suite, avec jeton et événements ;
- client TypeScript de la suite (`@suite-media/client`) et interface desktop Tauri + React
  (mode studio, scènes, sources, propriétés, enregistrement et direct).

Prochaines étapes (voir le projet Linear « Media · Maya Studio ») :

1. Backend GStreamer in-process (LGPL) avec encodeurs matériels, à la place du processus ffmpeg.
2. Vraies sources : caméras, capture d'écran et de fenêtre, micros, fichiers média, SRT/NDI entrants.
3. Audio : mixage, VU-mètres, synchro A/V.
4. Compositeur GPU wgpu (DirectX 12 / Metal / Vulkan).
5. Interface desktop : flux d'aperçu à faible latence (les moniteurs interrogent des captures JPEG pour l'instant).
6. Découverte mDNS des apps de la suite.

## Architecture

```
crates/
  suite-protocol/   contrat commun de la Suite Media (enveloppe, poignée de main, erreurs)
  studio-core/      modèle de scènes, opérations, compositeur
  studiod/          moteur sans interface : boucle de rendu, sorties, serveur WebSocket
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

## Lancer

Prérequis : Rust stable, `ffmpeg` dans le PATH (provisoire), Node 22+ pour le client de démo.

```sh
cargo run --release -p studiod -- --token monjeton
# dans un autre terminal
node tools/demo-client.mjs ws://127.0.0.1:4460 monjeton
```

Options de `studiod` : `--listen 127.0.0.1:4460`, `--token` (ou `STUDIO_TOKEN`),
`--recordings recordings`, `--width 1280 --height 720 --fps 30`.

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
GStreamer et FFmpeg en build LGPL, crates Rust MIT/Apache. Le backend provisoire
appelle un `ffmpeg` externe avec `libx264` (GPL) : il sert au développement seulement
et sera remplacé par GStreamer et les encodeurs matériels avant toute distribution.
