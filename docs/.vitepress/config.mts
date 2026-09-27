import { defineConfig } from 'vitepress'

// The VitePress site of the port, in two languages: `root` is the English site
// at /mars-rs/ and `zh` is the Chinese one at /mars-rs/zh/. One set of files
// per locale under `docs/` — nothing is translated at build time, and the two
// are the same pages with the same names.
//
// The site is served from a project page — <owner>.github.io/mars-rs/ — and not
// from the root of a domain, so every URL it writes carries `/mars-rs/` in
// front of it: `base` is what puts it there, and it is the one setting that has
// to agree with the workflow (a built site with no base in it resolves
// `/assets/...` against the root of github.io and comes back 404).
export default defineConfig({
  base: '/mars-rs/',
  // GitHub Pages resolves `/getting-started/` to `getting-started/index.html`,
  // and a link written `/getting-started` too, so `cleanUrls` is left off: the
  // `.html` it would strip is the cheapest way to keep every link working on
  // whatever serves the directory.
  locales: {
    root: {
      label: 'English',
      lang: 'en',
      title: 'mars-rs',
      description:
        'The xlog logging pipeline, the STN task model and the SDT network diagnosis of Tencent/mars, in Rust.',
      themeConfig: {
        langMenuLabel: 'Language',
        nav: [
          { text: 'Guide', link: '/getting-started' },
          { text: 'Configuration', link: '/configuration' },
          { text: 'Log files', link: '/log-files' },
          { text: 'Platforms', link: '/platforms/rust' },
        ],

        sidebar: [
          {
            text: 'Using mars-rs',
            items: [
              { text: 'Overview', link: '/' },
              { text: 'Getting started', link: '/getting-started' },
              { text: 'Configuration', link: '/configuration' },
              { text: 'Log files', link: '/log-files' },
            ],
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
    },

    zh: {
      label: '简体中文',
      lang: 'zh-Hans',
      title: 'mars-rs',
      description:
        'Tencent/mars 的 xlog 日志链路、STN 任务模型与 SDT 网络诊断的 Rust 实现。',
      themeConfig: {
        langMenuLabel: '语言',
        nav: [
          { text: '指南', link: '/zh/getting-started' },
          { text: '配置项', link: '/zh/configuration' },
          { text: '日志文件', link: '/zh/log-files' },
          { text: '各平台', link: '/zh/platforms/rust' },
        ],

        sidebar: [
          {
            text: '使用 mars-rs',
            items: [
              { text: '总览', link: '/zh/' },
              { text: '快速开始', link: '/zh/getting-started' },
              { text: '配置项', link: '/zh/configuration' },
              { text: '日志文件', link: '/zh/log-files' },
            ],
          },
          {
            text: '各平台',
            items: [
              { text: 'Rust', link: '/zh/platforms/rust' },
              { text: 'SwiftPM', link: '/zh/platforms/swift' },
              { text: 'Android', link: '/zh/platforms/android' },
              {
                text: 'Kotlin Multiplatform',
                link: '/zh/platforms/kotlin-multiplatform',
              },
              { text: 'C ABI', link: '/zh/platforms/c-abi' },
              { text: 'HarmonyOS', link: '/zh/platforms/harmonyos' },
            ],
          },
        ],

        socialLinks: [
          { icon: 'github', link: 'https://github.com/orangeboyChen/mars-rs' },
        ],

        editLink: {
          pattern:
            'https://github.com/orangeboyChen/mars-rs/edit/main/docs/:path',
          text: '在 GitHub 上修改本页',
        },

        search: { provider: 'local' },

        footer: {
          message: 'MIT，与上游项目一致。',
          copyright: 'Copyright © Tencent/mars 与 mars-rs 作者',
        },
      },
    },
  },
})
