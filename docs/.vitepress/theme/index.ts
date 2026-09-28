// The default theme, plus two things it does not do: the seven strings of it
// that no locale can reach, and the platform a reader picked, carried from the
// code group they picked it in to every other group of the site.
//
// `en` and `zh` in ../config.ts are every string the theme can be handed. They
// are not every string the theme writes: seven labels are English in the theme's
// own components, with no key in front of them — the names a screen reader hears
// for the three navigations and for the prev/next nav, the `aria-label` of the
// three bars that open the sidebar and of the "…" beside them, the caret that
// folds a section of the sidebar away, and the tooltip of the button that copies
// a code block. Nothing in the config can say anything about them, so what is
// here says it after the fact: once a page is on the screen, this walks it and
// writes the locale's word over the theme's.
//
// The table below is the seven, each with where to find it and what each locale
// calls it. `en` repeats what the theme says, and that repetition is the point:
// the walk runs on every page, so a reader who switches language gets the
// language they switched to and not the one they left.
// `document.documentElement.lang` is which one that is — VitePress writes the
// locale's `lang` into it and keeps it up to date as the reader moves.
//
// A MutationObserver is what says when to walk, because nothing else says it
// reliably: a navigation renders a new page into the same document, and the
// theme has no event on the way out of that. Every write is guarded by a
// comparison first — writing into a text node is itself a change to the DOM, and
// the observer is watching.
//
// Not one of the seven is a string a reader sees. That is what makes this the
// right shape for them: a page of prose translated by hand is worth a page of
// prose, and these are worth the seven lines it takes to say them in another
// language. A bump of vitepress is the moment to look here again — the day the
// theme gives these a key of their own, this file is the whole of what to delete.
import DefaultTheme from 'vitepress/theme'
import { applyPlatform, rememberPlatform } from './platform'

const CHROME: {
  // where the string lives in the rendered page
  selector: string
  // what to write: the named attribute, or — with none — the text of the node
  attribute?: 'aria-label' | 'title'
  // what each locale calls it, keyed by the locale's `lang`
  text: Record<string, string>
}[] = [
  {
    // the three bars that open the sidebar, below 768px
    selector: '.VPNavBarHamburger',
    attribute: 'aria-label',
    text: { en: 'mobile navigation', 'zh-Hans': '菜单' },
  },
  {
    // the "…" that opens the language menu, the appearance switch and the links
    selector: '.VPNavBarExtra .button',
    attribute: 'aria-label',
    text: { en: 'extra navigation', 'zh-Hans': '更多导航' },
  },
  {
    selector: '#main-nav-aria-label',
    text: { en: 'Main Navigation', 'zh-Hans': '主导航' },
  },
  {
    selector: '#sidebar-aria-label',
    text: { en: 'Sidebar Navigation', 'zh-Hans': '侧边栏导航' },
  },
  {
    // the label the prev/next nav is named by
    selector: '#doc-footer-aria-label',
    text: { en: 'Pager', 'zh-Hans': '分页导航' },
  },
  {
    selector: '.caret',
    attribute: 'aria-label',
    text: { en: 'toggle section', 'zh-Hans': '展开或收起本节' },
  },
  {
    // the tooltip of the copy button of a code block
    selector: 'button.copy',
    attribute: 'title',
    text: { en: 'Copy Code', 'zh-Hans': '复制代码' },
  },
]

function say(node: Element, attribute: string | undefined, text: string): void {
  if (attribute) {
    if (node.getAttribute(attribute) !== text) node.setAttribute(attribute, text)
  } else if (node.textContent?.trim() !== text) {
    node.textContent = text
  }
}

function walk(): void {
  const lang = document.documentElement.lang
  for (const { selector, attribute, text } of CHROME) {
    const label = text[lang]
    if (label) {
      document.querySelectorAll(selector).forEach((node) => say(node, attribute, label))
    }
  }

  // The same walk is the one that says a page is on the screen, and the groups
  // of it are what the reader's platform is handed to.
  applyPlatform()
}

export default {
  extends: DefaultTheme,
  enhanceApp() {
    // The build renders every page once in node, where there is no document to
    // walk and no reader to walk it for.
    if (typeof document === 'undefined') return

    // A pick of platform is a pick for the whole site, and one that outlives
    // the page it was made on: what listens for it is in ./platform.ts, and
    // what it does with the pick is the `applyPlatform` below.
    rememberPlatform()

    // Every batch of changes to the page is a walk: a navigation renders a new
    // page into the same document, and the writes below are guarded, so a walk
    // that finds nothing to do is the end of it.
    new MutationObserver(walk).observe(document.body, { childList: true, subtree: true })

    // The page the reader arrived on is already in the document and may never
    // mutate, so it needs a walk of its own — after the frame that mounts the
    // app and hydrates what the build wrote.
    requestAnimationFrame(walk)
  },
}
