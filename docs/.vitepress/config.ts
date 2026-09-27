import { defineConfig } from 'vitepress'
import type { DefaultTheme } from 'vitepress'

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
//
// Two languages is not only two sets of pages. VitePress itself has a voice —
// the labels of its own chrome, the words under the outline, the button that
// opens the search, the 404 — and every one of them is English until it is
// told otherwise. `en` and `zh` below are that voice: one object per locale,
// holding every string the theme can be handed, and each locale's `themeConfig`
// spreads its own over the shared settings. Nothing here is a partial
// translation: a reader who reads only one of the two never sees a word of the
// other except in the language menu, which is the one place both belong.
//
// One page is outside that: GitHub Pages answers every missing path with the one
// `404.html` a build writes, and there is only one of them, so it is the root
// locale's 404 a reader lands on even from a Chinese link. Each locale carries
// its own anyway, because what a locale's config says should not depend on
// where it is served from.

const REPO = 'https://github.com/orangeboyChen/mars-rs'
// `:path` is the file under `docs/`, so the same pattern opens the Chinese page
// the reader is on and not its English twin.
const EDIT_LINK = `${REPO}/edit/main/docs/:path`

// Every string of the theme's own chrome, in English. What is not in this list
// is not in the theme: these are the keys VitePress falls back to a default
// English string for, and the defaults are what a reader sees when a locale is
// silent. `i18nRouting` is the one that is not a string: a switch of language
// keeps the reader on the page they were reading, which is why the two trees
// carry the same file names.
const en: DefaultTheme.Config = {
  i18nRouting: true,
  outline: { level: [2, 3], label: 'On this page' },
  docFooter: { prev: 'Previous page', next: 'Next page' },
  lastUpdated: {
    text: 'Last updated',
    // `forceLocale` is what makes the date a date of this locale and not of
    // the reader's browser: without it the Chinese site prints a date in
    // whatever language the reader's `Intl` answers in.
    formatOptions: { dateStyle: 'long', forceLocale: true },
  },
  editLink: { pattern: EDIT_LINK, text: 'Suggest a change to this page' },
  langMenuLabel: 'Change language',
  returnToTopLabel: 'Return to top',
  sidebarMenuLabel: 'Menu',
  darkModeSwitchLabel: 'Appearance',
  lightModeSwitchTitle: 'Switch to light theme',
  darkModeSwitchTitle: 'Switch to dark theme',
  skipToContentLabel: 'Skip to content',
  notFound: {
    code: '404',
    title: 'Page not found',
    quote:
      'But if you don’t change your direction, and if you keep looking, you may end up where you are heading.',
    linkLabel: 'go to home',
    linkText: 'Take me home',
  },
  search: {
    provider: 'local',
    options: {
      translations: {
        // `buttonText` is printed on the button in the navbar and used as the
        // placeholder of the box the button opens.
        button: { buttonText: 'Search', buttonAriaLabel: 'Search' },
        modal: {
          displayDetails: 'Display detailed list',
          resetButtonTitle: 'Reset search',
          backButtonTitle: 'Close search',
          noResultsText: 'No results for',
          footer: {
            selectText: 'to select',
            selectKeyAriaLabel: 'enter',
            navigateText: 'to navigate',
            navigateUpKeyAriaLabel: 'up arrow',
            navigateDownKeyAriaLabel: 'down arrow',
            closeText: 'to close',
            closeKeyAriaLabel: 'escape',
          },
        },
      },
    },
  },
  socialLinks: [{ icon: 'github', link: REPO, ariaLabel: 'mars-rs on GitHub' }],
  footer: {
    message: 'MIT, like the upstream project.',
    copyright: 'Copyright © Tencent/mars and the mars-rs authors',
  },
}

// The same keys in Chinese. The search box is the one with a shape of its own:
// `noResultsText` is printed in front of the query — 没有找到 "query" — and the
// three of the footer are printed behind the key they name, so they are the
// words of a keyboard and not of an instruction.
const zh: DefaultTheme.Config = {
  i18nRouting: true,
  outline: { level: [2, 3], label: '本页目录' },
  docFooter: { prev: '上一页', next: '下一页' },
  lastUpdated: {
    text: '最后修改于',
    formatOptions: { dateStyle: 'long', forceLocale: true },
  },
  editLink: { pattern: EDIT_LINK, text: '在 GitHub 上修改本页' },
  langMenuLabel: '切换语言',
  returnToTopLabel: '回到顶部',
  sidebarMenuLabel: '菜单',
  darkModeSwitchLabel: '外观',
  lightModeSwitchTitle: '切换到浅色主题',
  darkModeSwitchTitle: '切换到深色主题',
  skipToContentLabel: '跳到正文',
  notFound: {
    code: '404',
    title: '页面不存在',
    quote: '如果不改变方向，并且一直找下去，你也许会走到你要去的地方。',
    linkLabel: '回到首页',
    linkText: '带我回首页',
  },
  search: {
    provider: 'local',
    options: {
      translations: {
        button: { buttonText: '搜索', buttonAriaLabel: '搜索文档' },
        modal: {
          displayDetails: '显示详细列表',
          resetButtonTitle: '清除搜索条件',
          backButtonTitle: '关闭搜索',
          noResultsText: '没有找到',
          footer: {
            selectText: '选择',
            selectKeyAriaLabel: '回车',
            navigateText: '切换',
            navigateUpKeyAriaLabel: '上箭头',
            navigateDownKeyAriaLabel: '下箭头',
            closeText: '关闭',
            closeKeyAriaLabel: 'Esc',
          },
        },
      },
    },
  },
  socialLinks: [{ icon: 'github', link: REPO, ariaLabel: 'mars-rs 在 GitHub 上' }],
  footer: {
    message: 'MIT，与上游项目一致。',
    copyright: 'Copyright © Tencent/mars 与 mars-rs 作者',
  },
}

export default defineConfig({
  base: '/mars-rs/',
  // GitHub Pages resolves `/getting-started/` to `getting-started/index.html`,
  // and a link written `/getting-started` too, so `cleanUrls` is left off: the
  // `.html` it would strip is the cheapest way to keep every link working on
  // whatever serves the directory.
  //
  // `lastUpdated` is turned on here and not per locale because what it turns on
  // is not the line of text — that is `lastUpdated.text`, and each locale has
  // its own — but the git timestamp behind it: VitePress reads one commit per
  // page to print a date at all, and it reads them only when this is set. A
  // build with a shallow clone has no commits to read, so the workflow that
  // builds this fetches the whole history.
  //
  // The empty object is what "on" is spelled like: VitePress takes a boolean
  // here as well, but the type it ships admits only the options object, so this
  // is the one spelling both agree on.
  themeConfig: {
    lastUpdated: {},
  },
  // One string of the markdown pipeline is English too, and it is the one with
  // the heading in it: every heading carries a link whose `aria-label` reads
  // `Permalink to "..."`. VitePress writes that label itself and hands no key to
  // it, but it spreads `markdown.anchor` over its own anchor options last, so
  // this generator is the one that runs: it writes the same link the theme
  // writes — same class, same `#slug`, same zero-width space in it — over the
  // heading's own inline tokens, with the locale's words around the title.
  // `state.env` is the page being rendered, and `localeIndex` on it is which
  // locale that page belongs to.
  markdown: {
    anchor: {
      permalink: (slug, _options, state, index) => {
        const inline = state.tokens[index + 1]
        const children = inline.children ?? []
        inline.children = children

        const link = new state.Token('link_open', 'a', 1)
        link.attrs = [
          ['class', 'header-anchor'],
          ['href', `#${slug}`],
          [
            'aria-label',
            state.env.localeIndex === 'zh'
              ? `“${inline.content}” 的永久链接`
              : `Permalink to "${inline.content}"`,
          ],
        ]

        const symbol = new state.Token('html_inline', '', 0)
        symbol.content = '&ZeroWidthSpace;'

        children.push(
          Object.assign(new state.Token('text', '', 0), { content: ' ' }),
          link,
          symbol,
          new state.Token('link_close', 'a', -1),
        )
      },
    },
  },
  locales: {
    root: {
      label: 'English',
      lang: 'en',
      title: 'mars-rs',
      description:
        'The xlog logging pipeline, the STN task model and the SDT network diagnosis of Tencent/mars, in Rust.',
      themeConfig: {
        ...en,
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
              { text: 'Flutter', link: '/platforms/flutter' },
              { text: 'React Native', link: '/platforms/react-native' },
              { text: 'The C ABI', link: '/platforms/c-abi' },
              { text: 'HarmonyOS', link: '/platforms/harmonyos' },
            ],
          },
        ],
      },
    },

    zh: {
      label: '简体中文',
      lang: 'zh-Hans',
      title: 'mars-rs',
      description:
        'Tencent/mars 的 xlog 日志链路、STN 任务模型与 SDT 网络诊断的 Rust 实现。',
      themeConfig: {
        ...zh,
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
              { text: 'Flutter', link: '/zh/platforms/flutter' },
              { text: 'React Native', link: '/zh/platforms/react-native' },
              { text: 'C ABI', link: '/zh/platforms/c-abi' },
              { text: 'HarmonyOS', link: '/zh/platforms/harmonyos' },
            ],
          },
        ],
      },
    },
  },
})
