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
- s'il plante, le shell le relance avec le même port et le même jeton (trois fois par
  minute au plus) et prévient l'interface par l'événement Tauri `studio-engine`.

## Écrans

- **Mode studio** : moniteurs Aperçu (vert) et Programme (rouge) côte à côte, bouton **Couper**.
- **Scènes** : créer, renommer, supprimer ; clic = aperçu, double-clic = programme.
- **Sources** de la scène en aperçu : ajouter (couleur, image, mire), masquer, monter /
  descendre, supprimer ; **Propriétés** : nom, couleur ou chemin, position, taille, opacité.
- **Sorties** : démarrer / arrêter l'enregistrement et le direct (serveur RTMP/SRT + clé),
  durée et images en temps réel.
- **Barre d'état** : connexion, canvas, cadence.
- Raccourcis : <kbd>Espace</kbd> ou <kbd>Entrée</kbd> = couper, <kbd>1</kbd>…<kbd>9</kbd> = scène en aperçu.

L'état est tenu à jour **uniquement par les événements** du studio (`applyStudioEvent`),
après un `GetStudio` à chaque (re)connexion : une action faite par Nino ou un autre
client apparaît immédiatement.

Les moniteurs interrogent `GetScreenshot` en JPEG environ 5 fois par seconde. C'est
provisoire : un vrai flux d'aperçu à faible latence (WebRTC, MJPEG ou mémoire partagée
côté Tauri) fera l'objet d'un ticket dédié.

## Lancer

Prérequis communs : Rust stable, Node 22+, `ffmpeg` dans le PATH (backend de sortie provisoire).
À la racine du dépôt : `npm install` puis `npm run build` (compile `@suite-media/client`).

### macOS

```sh
xcode-select --install          # outils de compilation, une fois
npm install && npm run build    # à la racine du dépôt
cd apps/desktop
npm run tauri dev
```

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

## Structure

```
src/
  connection.ts           d'où viennent l'URL et le jeton (Tauri, ?url=, VITE_*, écran de connexion)
  studio/StudioContext    client unique, état tenu par les événements, sorties, notifications
  studio/useScreenshot    moniteurs (interrogation JPEG)
  components/             moniteurs, scènes, sources, propriétés, sorties, fenêtres modales
test/                     tests Vitest (jsdom) : champs, raccourcis, sélection, erreurs fatales
src-tauri/
  src/lib.rs              lancement et relance du sidecar, commande studio_connection, arrêt à la fermeture
  tauri.conf.json         externalBin = binaries/studiod
scripts/prepare-sidecar.mjs
```

Le crate `src-tauri` est hors de l'espace de travail Cargo racine (il a son propre
`[workspace]`) pour que `cargo build` du moteur n'ait pas besoin de WebKit.
