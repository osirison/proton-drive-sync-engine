# Documentation site

The user-facing docs for Proton Drive Sync, built with [Astro](https://astro.build/) and
[Starlight](https://starlight.astro.build/). Content lives in `src/content/docs/`; the
sidebar and site config are in `astro.config.mjs`.

## Develop

```sh
cd website
npm install
npm run dev        # http://localhost:4321/proton-drive-sync-engine/
```

| Command | Action |
| --- | --- |
| `npm run dev` | Start the dev server with hot reload |
| `npm run build` | Build the static site into `dist/` |
| `npm run preview` | Serve the built `dist/` locally |
| `npm run check` | Type-check content and config |

## Deploy

Pushing to `main` with changes under `website/` triggers `.github/workflows/docs.yml`,
which builds and publishes to GitHub Pages at
`https://osirison.github.io/proton-drive-sync-engine/`. Enable Pages once under
**Settings → Pages → Source: GitHub Actions**.

The `site`/`base` are configurable via env vars for other hosts:

```sh
# root-hosted preview (no base path)
BASE_PATH=/ SITE_URL=http://localhost npm run build
```

## Notes

- Internal Markdown links are written root-relative (`/safety/deletions/`); a small rehype
  plugin in `astro.config.mjs` prepends the site `base` at build time so a project-pages
  deploy doesn't 404.
- The logo/favicon come from the repo's own `assets/icon.svg` — keep them in sync if the
  brand icon changes.
