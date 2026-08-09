// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { satteri } from '@astrojs/markdown-satteri';
import { defineHastPlugin } from 'satteri';

// Deployed to GitHub Pages as a project site:
//   https://osirison.github.io/proton-drive-sync-engine/
// Override `site`/`base` with env vars for a different host (e.g. a custom domain
// or a local preview at the root).
const site = process.env.SITE_URL ?? 'https://osirison.github.io';
const base = process.env.BASE_PATH ?? '/proton-drive-sync-engine';

/**
 * Astro does not prepend the site `base` to root-relative links written in
 * Markdown/MDX bodies (`[text](/safety/deletions/)`) — only Starlight's own
 * components (sidebar, cards, hero) are base-aware. This plugin rewrites
 * in-body `<a href="/…">` links to include the base, so a project-pages deploy
 * under `/proton-drive-sync-engine/` doesn't 404 on every internal link.
 * With `BASE_PATH=/` (a root preview) the prefix is empty and it's a no-op.
 *
 * A SÄTTERI HAST PLUGIN, NOT A REHYPE ONE. Astro made Sätteri the default
 * Markdown processor and stopped installing `@astrojs/markdown-remark`, so
 * `markdown.rehypePlugins` now throws at config time asking for that package.
 * Installing it would have "fixed" the build by dragging the whole site back
 * onto the unified processor — and Starlight has moved to Sätteri too
 * (`@astrojs/markdown-satteri`), so the two would then be rendering the same
 * pages through different pipelines.
 *
 * The port is close to a rename. `filter: ['a']` replaces walking the tree
 * looking for anchors — Sätteri filters on the Rust side, so only matched nodes
 * cross the boundary — and `ctx.setProperty` replaces assigning to
 * `node.properties`, because the node arrives `Readonly` and every mutation goes
 * through the context instead of the tree.
 */
function baseLinksPlugin(basePath) {
  const prefix = basePath === '/' ? '' : basePath.replace(/\/$/, '');
  if (!prefix) return null;
  return defineHastPlugin({
    name: 'base-links',
    element: {
      filter: ['a'],
      visit(node, ctx) {
        const href = node.properties && node.properties.href;
        if (
          typeof href === 'string' &&
          href.startsWith('/') &&
          !href.startsWith('//') &&
          href !== prefix &&
          !href.startsWith(prefix + '/')
        ) {
          ctx.setProperty(node, 'href', prefix + href);
        }
      },
    },
  });
}

const baseLinks = baseLinksPlugin(base);

export default defineConfig({
  site,
  base,
  trailingSlash: 'ignore',
  markdown: {
    processor: satteri({ hastPlugins: baseLinks ? [baseLinks] : [] }),
  },
  integrations: [
    starlight({
      title: 'Proton Drive Sync',
      description:
        'Bidirectional file sync between a local folder and Proton Drive — a Rust daemon, a control CLI, and a desktop app.',
      logo: {
        src: './src/assets/icon.svg',
        alt: 'Proton Drive Sync',
      },
      favicon: '/favicon.svg',
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/osirison/proton-drive-sync-engine',
        },
      ],
      editLink: {
        baseUrl:
          'https://github.com/osirison/proton-drive-sync-engine/edit/main/website/',
      },
      lastUpdated: true,
      tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 3 },
      customCss: ['./src/styles/custom.css'],
      sidebar: [
        {
          label: 'Start here',
          items: [
            { label: 'What it is', link: '/start/introduction/' },
            { label: 'Installation', link: '/start/installation/' },
            { label: 'Quick start', link: '/start/quick-start/' },
          ],
        },
        {
          label: 'Core concepts',
          items: [
            { label: 'How sync works', link: '/concepts/how-sync-works/' },
            { label: 'Change detection', link: '/concepts/change-detection/' },
            { label: 'Architecture', link: '/concepts/architecture/' },
          ],
        },
        {
          label: 'Safety',
          items: [
            { label: 'Deletions & the safeguard', link: '/safety/deletions/' },
            { label: 'Delete approval', link: '/safety/delete-approval/' },
            { label: 'Conflicts', link: '/safety/conflicts/' },
            { label: 'Dry-run preview', link: '/safety/dry-run/' },
          ],
        },
        {
          label: 'Daemon — proton-syncd',
          items: [
            { label: 'Command reference', link: '/daemon/reference/' },
            { label: 'Configuration', link: '/daemon/configuration/' },
            { label: 'Selective sync', link: '/daemon/selective-sync/' },
            { label: 'Logging', link: '/daemon/logging/' },
            { label: 'Running as a service', link: '/daemon/running-as-a-service/' },
          ],
        },
        {
          label: 'Control CLI — proton-sync',
          items: [{ label: 'Command reference', link: '/cli/reference/' }],
        },
        {
          label: 'Desktop app',
          items: [
            { label: 'Overview', link: '/desktop/overview/' },
            { label: 'Screens', link: '/desktop/screens/' },
            { label: 'Tray & notifications', link: '/desktop/tray/' },
            { label: 'File-manager emblems', link: '/desktop/emblems/' },
          ],
        },
        {
          label: 'Distribution',
          items: [{ label: 'Native packages', link: '/distribution/packages/' }],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Troubleshooting', link: '/reference/troubleshooting/' },
            { label: 'FAQ', link: '/reference/faq/' },
            { label: 'Development', link: '/reference/development/' },
          ],
        },
      ],
    }),
  ],
});
