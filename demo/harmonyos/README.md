# The HarmonyOS demo

A whole hvigor project: one ability, one page, one button, six records into a
`.xlog` file in the app's `filesDir`.

Open it in DevEco Studio — **File ▸ Open** on this directory — and run it, or
build it from the command line:

```bash
hvigorw assembleHap --mode module -p module=entry@default -p product=default
```

## The one thing that is not here

`marsrs-harmonyos-xlog` is a HAR, and publishing to ohpm is not switched on yet,
so `ohpm install marsrs-harmonyos-xlog` resolves nothing. The release carries the
same archive, and it goes in `libs/`:

```bash
gh release download v0.1.0-alpha.3 --pattern 'marsrs-harmonyos-xlog-*.har' \
    --dir demo/harmonyos/libs
mv demo/harmonyos/libs/marsrs-harmonyos-xlog-*.har \
    demo/harmonyos/libs/marsrs-harmonyos-xlog-1.0.0.har
ohpm install
```

`libs/` is ignored, because it is a download and not a source. The rename is
what makes the download and `entry/oh-package.json5` the same file, and the
`1.0.0` it renames to is not the version of the release: a release asset carries
the version of its release — `marsrs-harmonyos-xlog-0.1.0-alpha.3.har` today —
while a `file:` dependency names one file, so the download is the half that
changes its name. `1.0.0` is the version the package will carry once it is on
ohpm, and the name the repository's own build writes the HAR of a checkout
under. The line an app writes once the package *is* on ohpm is
`"marsrs-harmonyos-xlog": "^1.0.0"` in
`entry/oh-package.json5`, and nothing else in the module changes.

## The two numbers that are the SDK's

`compatibleSdkVersion` in `build-profile.json5` and `modelVersion` in
`oh-package.json5` and `hvigor/hvigor-config.json5` are the SDK's own, and not
the app's: the first is `<platformVersion>(<apiVersion>)` of the SDK the build
runs against, and the second is its platform version — the same string in both
files, because hvigor reads it out of each and compares. They are written here as
HarmonyOS 5.1.0 Release, API 18, which is what
`scripts/fetch_deveco_cli.sh` pins for the repository's own builds. A newer SDK
names its own in the build log, and the fix is to put that number in both files.

## What to look at

- `Xlog.open(config)` in `entry/src/main/ets/model/LogStore.ets` — the same call
  it is in Kotlin, in Swift and in Dart, and the one that throws: an app that
  lets it reach the ability's `onCreate` is an app that dies on a device whose
  storage is full, so the reason is kept and the app runs without a log.
- `context.filesDir` — the directory the system gives the app, and the one it
  may write without a permission. It is a thing the ability has and a page does
  not, which is why the appender is opened in `onCreate` and not in the page.
- `xlog.log(level, tag, message)` — one method per level as well, `xlog.v` and
  friends, and `xlog.flushNow()` for the call that waits.
- `entry/src/main/resources/base/profile/main_pages.json` — the pages
  `router.pushUrl` may name, and the one `windowStage.loadContent` loads first.
  It is the only strict JSON in the module and not a `.json5`, so it takes no
  comments: hvigor's parser rejects the file outright, in `ProcessResource`,
  before a line of ArkTS is compiled.
- `closeLogStore()` in `onDestroy` — the last moment an app is told anything at
  all, and with an async appender the last moment its writer thread is
  guaranteed to be running.
- `flushLogStore()` in `EntryAbility.onBackground` — the app leaves the screen,
  which is the last moment before the process can be ended without a word, so
  the records still in the cache are taken to the file there and not only at
  `onDestroy`.

The icon in `AppScope/resources/base/media/` and the one in `entry/src/main/resources/base/media/`
are single-colour placeholders, so that the project builds as it stands; an app
that ships replaces them.
