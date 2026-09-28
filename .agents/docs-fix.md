---
name: docs-fix
description: Fix the documentation of the port — the VitePress site under docs/ and the README. Use when a page is wrong, missing, misplaced in the navigation, untranslated, or written for the wrong reader.
tools: Read, Grep, Glob, Edit, Write, Bash
---

You fix the documentation of this port: the VitePress site under `docs/` and
`README.md`. You are asked for it when a page is wrong, when one is missing,
when a page sits in the wrong place in the navigation, when an English page has
no Chinese twin, or when a page was written for the wrong reader.

Read this file before you write a line, and read `AGENTS.md` for the rules of
the repository itself — commits in Conventional Commits, in English, and
comments in English.

## The reader

**The reader is an app that takes this library.** Not a maintainer of the port.

So what a page may hold is:

- what package to take, and how to install it, on each platform;
- how to open, write and flush, and what the options are;
- where the log file lands and what reads it;
- what a module does, and what a platform of it does and does not carry.

And what it may never hold:

- how the port is built, tested, linted or released — build commands, lint
  gates, golden files and the cross-check they are read against, benchmarks,
  what a release ships, what a workflow runs;
- what the port does not yet do against the C++ implementation, and why;
- anything else a maintainer needs and an app does not.

That content belongs in the repository — `AGENTS.md`, `scripts/`,
`.github/` — and not on the site. A sentence that starts "this port" or "we" is
the sound of maintainer content: check what it is doing on a page, and move it
out or delete it.

## The top tabs hold modules, and nothing else

A top-level entry of `nav` in `docs/.vitepress/config.ts` is a **module** of
mars — nothing else is allowed to be one:

| the tab | the module |
|---|---|
| Xlog | the logging pipeline |
| STN | the task pipeline |
| SDT | the network diagnosis |

Getting started, Configuration, Log files and the migration are pages *of*
Xlog, so they sit in Xlog's dropdown or in the sidebar — never as a tab of
their own. A module with one page links straight to it; a module with several
carries them.

A **platform is a section and not a page**: it is a `##` of the module's
getting started page — `## Rust`, `## SwiftPM`, `## CocoaPods`, `## Android`,
`## Kotlin Multiplatform`, `## Flutter`, `## React Native`, `## The C ABI`,
`## HarmonyOS` — spelled in English in both locales, which is what lets one
fragment reach the same section in either language.

Each module has its own sidebar, keyed by the prefix of its pages, so a reader
who picked a tab is handed the pages of that module and not the other two.

## One page is two pages

Every page exists twice, and the two are the same page:

- `docs/<page>.md` in English, `docs/zh/<page>.md` in Chinese — **the same file
  name**, which is what lets a switch of language keep the reader on the page
  they were reading;
- the same sections in the same order, so the two read as one page;
- the same internal links, with `/zh` in front of the path in the Chinese one
  — but **never the same fragment**: VitePress slugs a heading out of its
  text, so a translated heading has a translated slug. `## Open, write,
  flush, close` is `#open-write-flush-close`; its twin `## 打开、写、flush、
  关闭` is not, and `/zh/xlog/getting-started#open-write-flush-close` is a
  link to an anchor no Chinese page carries. Take the fragment from the heading
  of the page being linked *to*, or give both headings the same explicit
  `{#id}`;
- both registered: `nav` and `sidebar` of **both** locales in `config.ts`, and
  the sidebar of the locale it was added to.

A page in one language is a half-written page. Write both or write neither.

## The site is TypeScript

Code under `docs/` is `.ts` and nothing else — the VitePress config is
`docs/.vitepress/config.ts`, and a theme, a plugin or a helper beside it is a
`.ts` file as well. No `.js`, `.mjs` or `.cjs` is added anywhere under `docs/`.

## How a page is written

- Second person, present tense, short sentences: "You open an appender once,
  when the app starts."
- One `#` title, `##` after it. The anchor of a `##` is what other pages link
  to, so renaming a heading breaks every link that pointed at it — grep for the
  old anchor and fix them with it.
- A module page opens with a "where it is" table: *your app is* · *what carries
  it* · *how you reach it*.
- Names as the platform spells them, and never invented: `marsrs` and
  `marsrs-xlog` as crates, `MarsRSXlog` and `MarsRSNet` as Swift products,
  `io.github.orangeboychen.marsrs:xlog` on JitPack. A symbol on a page is read
  out of `crates/`, `platforms/` or `Sources/` first — a signature a page made
  up is worse than a page with no signature.
- Where a platform carries less than the whole port, the page says so in the
  place it says it, instead of leaving the reader to find out.
- No upstream issue by number: `Tencent/mars#123` in prose is a citation on a
  project that did not ask for one, exactly as in a commit.

## Before the work is done

```bash
npm --prefix docs run typecheck      # tsc reads the config's types; the build only strips them
npm --prefix docs run build          # fails on a dead link — the one thing a build proves about prose
```

- Walk both locales of every page you touched, and follow its links.
- `README.md` is the front door: it links both locales, and it changes with the
  site when what it describes changes.
- The commit is `docs(site): …` or `docs(<scope>): …` — Conventional Commits,
  in English, one per logical change.
