// Lint gate for the Tauri frontend in gui/src.
//
// gui/src is served RAW by the webview (tauri.conf.json: "frontendDist": "../src", no
// beforeBuildCommand, no bundler). Nothing rewrites these files between here and the browser, so a
// bad import specifier is not a build error — it is a blank window at runtime, reproducible only by
// launching the app. That is the bug class this gate exists to catch; everything else is secondary.
//
// Every rule is "error": this runs in CI as a gate, and warnings nobody has to fix are noise. (It is
// also why the import-x plugin is registered by hand rather than via importX.flatConfigs.recommended,
// which ships several rules at "warn" — including one that fires on this file's own `importX` import.)

import js from "@eslint/js";
import globals from "globals";
import importX from "eslint-plugin-import-x";

const TAURI_FACADE_MESSAGE =
  "Reach the backend through the api facade in js/api.js, not the __TAURI__ injection — direct access breaks the browser-preview path.";

export default [
  // Patterns here and below are `**/`-prefixed on purpose. ESLint resolves `files`/`ignores`
  // against the working directory, not against this file, so a bare "src/js/api.js" silently
  // stops matching when someone points an editor at the repo root with an explicit
  // overrideConfigFile — which would INVERT the facade exemption below, flagging the one file
  // allowed to touch the injection while un-ignoring generated code. The prefix matches under
  // either root. Run from gui/ (as the npm scripts and CI do) for the supported behaviour.
  { ignores: ["**/src-tauri/target/**", "**/src-tauri/gen/**", "**/gui-core/**"] },

  js.configs.recommended,

  {
    // Matches the extension set ESLint lints by default. `**/*.js` alone would leave a future
    // .mjs/.cjs file with js.configs.recommended only — no browser globals and, worse, none of the
    // import-x rules, which is precisely the gap this config exists to close.
    files: ["**/*.{js,mjs,cjs}"],
    plugins: { "import-x": importX },
    languageOptions: {
      ecmaVersion: 2023,
      sourceType: "module",
      // Browser globals only. __TAURI__ is deliberately NOT declared: the code always reaches it as
      // a property of `window` (js/api.js), never as a bare identifier, so declaring it would be
      // dead config — and worse, it would silence no-undef on a bare `__TAURI__`, which is exactly
      // the facade bypass that breaks the browser-preview path.
      globals: { ...globals.browser },
    },
    rules: {
      // ---- module graph: the no-bundler blank-screen class ----
      // A specifier that does not resolve on disk (typo, moved file, wrong depth).
      "import-x/no-unresolved": "error",
      // The killer: `from "./api"` loads under a bundler and 404s in a webview. no-unresolved does
      // NOT catch this — the node resolver happily finds ./api.js — so this rule is load-bearing,
      // not decorative. Package specifiers are exempt; this config file imports three of them.
      "import-x/extensions": ["error", "always", { ignorePackages: true }],
      // Importing a name the target module does not export -> undefined at runtime, not an error.
      "import-x/named": "error",
      // Same, for `import * as store` member access (app.js dereferences store.setLedgerFilter).
      "import-x/namespace": "error",
      // Default-importing a module that has no default export.
      "import-x/default": "error",
      // Two exports of the same name from one module.
      "import-x/export": "error",
      // A cycle in real ESM is a TDZ ReferenceError at load time, not a lazy-init warning.
      "import-x/no-cycle": ["error", { ignoreExternal: true }],
      "import-x/no-self-import": "error",
      "import-x/no-useless-path-segments": ["error", { noUselessIndex: true }],
      "import-x/no-duplicates": "error",
      // `export let` hands importers a live binding that mutates under them.
      "import-x/no-mutable-exports": "error",

      // ---- dead code / undefined identifiers ----
      // no-undef comes from js.configs.recommended. Here we only teach no-unused-vars the
      // underscore convention this codebase already uses (catch (_), mockInvoke(cmd, _args)).
      "no-unused-vars": [
        "error",
        {
          // "after-used" (ESLint's default) is deliberate: settings.js threads a uniform
          // (container, ctx, ...) signature through ~15 helpers and some of them read neither.
          // Leading placeholders kept for signature shape are not dead code; a trailing parameter
          // nobody reads is, and that is still caught.
          args: "after-used",
          argsIgnorePattern: "^_",
          varsIgnorePattern: "^_",
          caughtErrors: "all",
          caughtErrorsIgnorePattern: "^_",
          destructuredArrayIgnorePattern: "^_",
          ignoreRestSiblings: true,
        },
      ],

      // ---- correctness ----
      "no-shadow": "error",
      "no-var": "error",
      "prefer-const": "error",
      // `== null` is a deliberate idiom here (components.js: `val != null`); everything else strict.
      eqeqeq: ["error", "always", { null: "ignore" }],
      // conflicts.js hand-writes diff loops with manual index advancement.
      "no-unmodified-loop-condition": "error",
      "no-self-compare": "error",
      // A "${x}" that never got its backticks — this codebase builds most strings from templates.
      "no-template-curly-in-string": "error",
      "no-promise-executor-return": "error",
      // `node.hidden;` instead of `node.hidden = false;` — a silent no-op in DOM-mutation code.
      "no-unused-expressions": "error",
      // A `default:` clause that is not last does not run last.
      "default-case-last": "error",

      // ---- webview-specific ----
      // alert/confirm/prompt block the WebKitGTK main loop (the failure mode behind #142/#143) and
      // are not reliably rendered by the Tauri webview at all.
      "no-alert": "error",
      // The shipped webview has no visible console, so a leftover debug log is write-only. Genuine
      // diagnostics (api.js reports a failed tray-navigate listener registration) stay allowed.
      "no-console": ["error", { allow: ["error", "warn"] }],
      // Architectural invariant, stated in js/api.js and js/app.js: every backend call goes through
      // the api facade so the same frontend runs in Tauri and in plain-browser design preview.
      // Reaching the injection directly throws at module evaluation in browser preview (nothing
      // injects __TAURI__ there), which is a blank screen. api.js itself is exempted below.
      //
      // All four spellings are covered deliberately: `window.__TAURI__`, `window["__TAURI__"]`,
      // `globalThis`/`self` in place of `window`, and `const { __TAURI__ } = window`. Guarding only
      // the dotted `window.` form would leave three trivial ways to bypass an invariant whose whole
      // value is that it cannot be bypassed by accident.
      "no-restricted-syntax": [
        "error",
        {
          selector: "MemberExpression[object.name=/^(window|globalThis|self)$/][property.name='__TAURI__']",
          message: TAURI_FACADE_MESSAGE,
        },
        {
          selector: "MemberExpression[object.name=/^(window|globalThis|self)$/][property.value='__TAURI__']",
          message: TAURI_FACADE_MESSAGE,
        },
        {
          selector: "ObjectPattern > Property[key.name='__TAURI__']",
          message: TAURI_FACADE_MESSAGE,
        },
      ],
    },
  },

  // js/api.js is the one file allowed to touch the Tauri injection: it is the facade.
  {
    files: ["**/src/js/api.js"],
    rules: { "no-restricted-syntax": "off" },
  },

  // This file is a Node ES module, not browser code. Without this it is linted under browser-only
  // globals, so a `process.env` guard — or a debug `console.log` while editing the config — fails
  // CI with "'process' is not defined", a confusing error in a plainly-Node file.
  {
    files: ["**/eslint.config.js"],
    languageOptions: { globals: globals.nodeBuiltin },
    rules: { "no-console": "off" },
  },
];
