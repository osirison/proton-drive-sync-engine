# Bundled fonts (IBM Plex)

The design bundles **IBM Plex Sans** (UI text) and **IBM Plex Mono** (paths, globs, field names,
timestamps, ids, counts) locally — no runtime webfont (the Tauri CSP forbids external font hosts).

Until the `.woff2` files are dropped in here, `styles/tokens.css` falls back to `system-ui` /
`ui-monospace`, so the layout renders correctly — only the exact typeface differs.

## To bundle the real fonts (OFL, from @fontsource/ibm-plex-sans + @fontsource/ibm-plex-mono)

Drop these Latin `.woff2` files into this directory:

```
ibm-plex-sans-400.woff2   ibm-plex-sans-500.woff2
ibm-plex-sans-600.woff2   ibm-plex-sans-700.woff2
ibm-plex-mono-400.woff2   ibm-plex-mono-500.woff2   ibm-plex-mono-600.woff2
```

Then uncomment the `@font-face` block in `styles/tokens.css`. (This environment had no network
access to fetch them during the foundation build.)
