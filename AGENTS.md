# AGENTS.md

The rules of this repository, for every agent that works in it and every person
who does. `CLAUDE.md` is a symlink to this file, so what is written here is
written once: there is one set of rules, and not two to keep in step.

Two of them are short enough to say before the rest:

- **A commit is a Conventional Commit, in English.**
- **A comment is in English.**

Everything below is what those two mean here, and what a change is expected to
look like when it is pushed.

## Commits

One line of `type: summary`, or of `type(scope): summary` when the change has a
scope — an empty line, then a body that says why.

### The summary

- English, in the present tense, and written as what the code does now rather
  than as an order to a reader: `a pull request is labelled by what it
  touches`, and not `label a pull request by what it touches`. This history
  speaks that way from one end of it to the other — `the five platform trees
  move under platforms/`, `Xlog is an appender you build and write through` —
  and a summary that reads like the ones beside it is the point.
- No capital letter after the colon, and no full stop at the end.
- One line, and a short one: the summaries of this history run to about 70
  characters, and past 80 a reader of a log is reading a wrapped line.
- `type` is one of `feat`, `fix`, `docs`, `ci`, `refactor`, `test`, `build`,
  `perf`, `deps`, `chore`. What the commit *did* decides it, not how big it
  was: a `feat` is a behaviour an app can now reach, a `fix` is one it can
  now reach correctly, a `refactor` changes no behaviour at all, and `deps`
  is a bump of a dependency and of nothing else.
- `scope` is the piece of the port the change is in — `xlog`, `stn`, `sdt`,
  `comm`, `ffi`, `jni` — or the platform it ships on — `android`, `swift`,
  `kmp`, `harmony`, `flutter`, `react-native`. A change that is the whole
  repository's carries none, and the history has those too: `ci: a run starts
  on every push, and it is the jobs that skip`.

### The body

- English, wrapped at 76 columns like every comment in the tree.
- Why, and not what: the diff already says what changed. What it cannot say is
  why the other shape was wrong, what the reader of the log would otherwise
  have to work out again, and what a reviewer would ask.
- No "Signed-off-by", no trailer but the one a tool put there.

### And

- One logical change per commit. A fix and the tidy-up it noticed are two
  commits, and a rename and the behaviour it unblocks are two as well.
- Never name an upstream issue by number — `Tencent/mars#123` — in a commit
  message or in the body of a pull request. Doing so posts a permanent event on
  that issue, on a project that did not ask for it. Describe the issue, or
  link a discussion; do not cite one.
- A commit is not a pull request. One branch is one pull request, however many
  commits it carries, and the pull request is where the change is described for
  a reader who did not write it.

## Comments, names and prose

- A comment is in English, in every language in this tree: Rust, Kotlin, Swift,
  the C headers, TypeScript, shell and YAML alike.
- A comment says why. A line that restates the line under it is a line a reader
  has to read twice — the code already said it once.
- A doc comment (`///`, KDoc, Swift's `///`) is written for the caller: what the
  item gives them, what it asks of them, and what happens when they do not.
- No Chinese in a source file. Chinese is written in `docs/zh/**` and in the
  locale strings the site is translated with — those live under
  `docs/.vitepress/`, where `config.ts` and `theme/index.ts` between them
  hold every string of the theme's own chrome — and nowhere else in the tree.
- Names are English too, and they are spelled the way the platform spells them:
  `marsrs` and `marsrs-xlog` as crates, `MarsRSXlog` and `MarsRSNet` as Swift
  products, `io.github.orangeboychen.marsrs` as a Maven group.

## Before a change is pushed

Every gate in this repository is strict: a finding is fixed in the source, and
not suppressed. No suppression is *added* without a line beside it saying why
the rule is wrong for that one call. The ones already in the tree are
deliberate and stay: `#[allow(unsafe_code)]` on the one mapping in
`marsrs-appender` that cannot be written without `unsafe` — the crate denies
it outright, and the invariant `memmap2` needs is argued beside it — and
`@Suppress("DEPRECATION")` where an Android API has no replacement below the
version the module supports.

- Rust: `cargo fmt --all`, then `cargo clippy` over the workspace.
- The site: `npm --prefix docs run typecheck`, then `npm --prefix docs run
  build`.
- Kotlin: ktlint and detekt. Swift: SwiftLint, in strict mode.

## The documentation

`docs/` and `README.md` are read by an app that takes this library, so what
they hold is how to install it and how to use it — never how the port is
developed. What a page may say, and where it goes in the site, is written down
for the agent that keeps it: `.agents/docs-fix.md`.
