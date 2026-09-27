import { defineConfig } from 'vitepress'

// The VitePress site of the port. It is served from a project page —
// <owner>.github.io/mars-rs/ — and not from the root of a domain, so every URL
// it writes carries `/mars-rs/` in front of it: `base` is what puts it there,
// and it is the one setting that has to agree with the workflow (a built site
// with no base in it resolves `/assets/...` against the root of github.io and
// comes back 404).
export default defineConfig({
  title: 'mars-rs',
  description:
    'Rust implementation of Tencent/mars: the xlog logging pipeline, the STN task model and the SDT network diagnosis.',
  base: '/mars-rs/',
  // GitHub Pages resolves `/getting-started/` to `getting-started/index.html`
  // but a link written `/getting-started` too, so `cleanUrls` is left off: the
  // `.html` it would strip is the cheapest way to keep every link working on
  // whatever serves the directory.
  themeConfig: {
    nav: [
      { text: 'Guide', link: '/getting-started' },
      { text: 'The format', link: '/format/' },
      { text: 'Performance', link: '/performance' },
      { text: 'Platforms', link: '/platforms/rust' },
      { text: 'Project', link: '/project/build' },
    ],

    sidebar: [
      {
        text: 'Introduction',
        items: [
          { text: 'Overview', link: '/' },
          { text: 'Getting started', link: '/getting-started' },
        ],
      },
      {
        text: 'The .xlog format',
        items: [
          { text: 'Pinned by golden files', link: '/format/' },
          { text: 'Cross-checked the other way', link: '/format/cross-check' },
        ],
      },
      {
        text: 'Performance',
        items: [{ text: 'What a record costs', link: '/performance' }],
      },
      {
        text: 'Platforms',
        items: [
          { text: 'Rust', link: '/platforms/rust' },
          { text: 'SwiftPM', link: '/platforms/swift' },
          { text: 'Android', link: '/platforms/android' },
          {
            text: 'Kotlin Multiplatform',
            link: '/platforms/kotlin-multiplatform',
          },
          { text: 'The C ABI', link: '/platforms/c-abi' },
          { text: 'HarmonyOS', link: '/platforms/harmonyos' },
        ],
      },
      {
        text: 'The project',
        items: [
          { text: 'Build, test and lint', link: '/project/build' },
          { text: 'What a release ships', link: '/project/packages' },
        ],
      },
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/orangeboyChen/mars-rs' },
    ],

    editLink: {
      pattern:
        'https://github.com/orangeboyChen/mars-rs/edit/main/docs/:path',
      text: 'Suggest a change to this page',
    },

    search: { provider: 'local' },

    footer: {
      message: 'MIT, like the upstream project.',
      copyright: 'Copyright © Tencent/mars and the mars-rs authors',
    },
  },
})
