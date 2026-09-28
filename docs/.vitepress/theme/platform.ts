// The platform a reader picked, carried from the code group they picked it in
// to every other group of the site.
//
// One of these pages answers a question — how do I open an appender, how do I
// read a log file — in eight platforms at once, and the reader who asks it asks
// four pages of it in a row. They are on Android for the first block and on
// Android for the rest of them, so a tab picked once is a tab that should not
// have to be picked again. VitePress keeps no state here of its own: a group is
// a row of radios and the blocks beside them, `active` is the one on show, and
// on a page the reader arrives at the first block is the one on show again.
//
// So what is here remembers the pick, in `localStorage` — a reader who comes
// back tomorrow comes back to the platform they were on — and hands it to every
// group on the page that carries it. Nothing is looked up once: the groups are
// not in the document yet when this runs, and after a navigation they are not
// the same nodes, so a pick is listened for on the document and the groups are
// asked for again each time.
//
// The names below are the words the pages are written with, and they are the
// same words in the two languages: the Chinese pages name a platform in
// English, the way the sidebar does. Not every group is a row of platforms —
// the CLI page groups two ways of installing it — and a group with no tab of
// the platform a reader is on is left where it is, there being nothing else to
// do with it.
//
// `mars-rs:` is in front of the key because GitHub Pages serves every project
// of this owner from the one origin, and so from the one `localStorage`.

// One line per platform, holding every name a group can call it by: an app on
// Apple is named on these pages by how it takes the package — SwiftPM,
// CocoaPods, or neither, which is the tab called Swift — and the three are one
// platform to the reader. A group is handed the first of the names it carries.
const PLATFORMS: string[][] = [
  ['Rust'],
  ['Swift', 'SwiftPM', 'CocoaPods'],
  ['Android'],
  ['Kotlin Multiplatform'],
  ['Flutter'],
  ['React Native'],
  ['C'],
]

// GitHub Pages serves every project of this owner from the one origin, and so
// from the one `localStorage`: the prefix is what makes this key ours.
const KEY = 'mars-rs:platform'

function platform(name: string): string[] | undefined {
  return PLATFORMS.find((names) => names.includes(name))
}

// A browser is allowed to refuse: Safari with its cookies off throws at both
// calls below, and a site that cannot remember is still a site that reads — in
// this one the pick lasts for the page the reader is on and no longer.
function stored(): string | null {
  try {
    return localStorage.getItem(KEY)
  } catch {
    return null
  }
}

function keep(name: string): void {
  try {
    localStorage.setItem(KEY, name)
  } catch {
    // no longer than the page, then
  }
}

// What a tab is labelled with. `data-title` is where VitePress writes the
// title of a block; the text of the label is the same words, and what a group
// is written with in a page that carries none.
function nameOf(tab: HTMLInputElement): string | undefined {
  const labels = tab.closest('.vp-code-group')?.querySelectorAll('label')
  const label = Array.from(labels ?? []).find((each) => each.htmlFor === tab.id)
  return label?.dataset.title || label?.textContent?.trim()
}

// `active` is the whole of what VitePress's own click handler moves, so this
// moves the same thing and leaves the rest of the group alone.
function select(tab: HTMLInputElement): void {
  const group = tab.closest('.vp-code-group')
  if (!group) return

  const blocks = group.querySelector('.blocks')
  const next = blocks?.children[Array.from(group.querySelectorAll('input')).indexOf(tab)]
  if (!next || next.classList.contains('active')) return

  blocks?.querySelector('.active')?.classList.remove('active')
  next.classList.add('active')
  // The radios of a group share a name, so checking this one is what leaves
  // the tab it replaces unchecked.
  tab.checked = true
}

let picked: string | null = null

// Hands the platform to every group on the page that carries it. Called on a
// pick, and again on every page: the walk in ./index.ts is what says when a
// page is there.
export function applyPlatform(): void {
  const names = picked ? platform(picked) : undefined
  if (!names) return

  document.querySelectorAll('.vp-code-group').forEach((group) => {
    const tab = Array.from(group.querySelectorAll('input')).find((input) =>
      names.includes(nameOf(input) ?? ''),
    )
    if (tab) select(tab)
  })
}

// A click on a tab is the reader saying which platform they are on. It is
// listened for on the document because the tab it lands on is not there yet:
// every page renders its own groups, into this document.
export function rememberPlatform(): void {
  picked = stored()

  document.addEventListener('click', (event) => {
    const tab = event.target
    if (!(tab instanceof HTMLInputElement) || !tab.matches('.vp-code-group input')) return

    const name = nameOf(tab)
    if (!name || !platform(name)) return

    picked = name
    keep(name)
    applyPlatform()
  })
}
