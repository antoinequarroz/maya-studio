# Site maya-studio.postmelo.ch

La page publique de Maya Studio : présentation, captures, API, plateformes, la suite et l'inscription à la bêta.

Fichiers statiques, sans build : `index.html`, `styles.css`, `main.js` et `assets/`. Le design reprend les jetons de la suite (Lamio, Cairn, Metricove) : mêmes fonds, mêmes polices (Geist, Geist Mono, Instrument Serif), même grain, et une couleur à soi. Maya Studio prend le rouge du voyant REC (`--accent`).

## Voir en local

```bash
cd site
python3 -m http.server 8080
```

Puis http://localhost:8080.

## Mettre en ligne sur le VPS

Pas encore fait. Même schéma que Cairn : le Caddy de Metricove sert la page.

1. **DNS** : un enregistrement **A** `maya-studio` dans la zone `postmelo.ch` (Infomaniak) vers l'IP du VPS. Vérifier avec `dig +short maya-studio.postmelo.ch`.
2. **Fichiers** : copier `site/` sur le VPS, par exemple dans `~/metricove/deploy/sites/maya-studio/`, et monter ce dossier dans le conteneur Caddy (`./sites:/srv/sites:ro` dans son `docker-compose.yml`).
3. **Caddy** : dans `~/metricove/deploy/Caddyfile`, ajouter :

   ```caddy
   maya-studio.postmelo.ch {
   	encode zstd gzip
   	header Strict-Transport-Security "max-age=31536000"
   	root * /srv/sites/maya-studio
   	file_server
   }
   ```

   Puis `docker compose exec caddy caddy reload --config /etc/caddy/Caddyfile`.

## Inscription à la bêta

Le formulaire lit deux attributs de `<form id="beta-form">` dans `index.html` :

- `data-endpoint` : une URL qui accepte un POST JSON `{email, platform, source}` ;
- `data-mailto` : une adresse, à défaut, qui ouvre un e-mail prêt à partir.

Sans l'un ni l'autre, le formulaire annonce que les inscriptions ouvrent bientôt et n'envoie rien.

## Plus tard

Les cartes « Téléchargements », « Documentation de l'API » et « Notes de version » (`#telechargements`, `#documentation`, `#notes-de-version`) deviennent des liens quand leurs pages existent.
