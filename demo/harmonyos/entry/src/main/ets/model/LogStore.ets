// The appender of the HarmonyOS demo: the one object that owns the `Xlog`,
// opened when the ability is created and closed when it is destroyed.
//
// It is the same three things every demo in `demo/` does — open, write one
// record at every level, flush — under the one name the port gives that call on
// every platform: `Xlog.open(config)`. What is HarmonyOS's is the directory the
// files go in, which is `context.filesDir` and never a path an app writes out
// for itself, and the split of the work across the ability's own lifecycle,
// which is where an app learns it is starting and stopping.

import { common } from '@kit.AbilityKit';
import { AppenderMode, CompressMode, LogLevel, Xlog } from 'marsrs-harmonyos-xlog';

/** The prefix of every file: `marsrs_YYYYMMDD.xlog`. */
const NAME_PREFIX: string = 'marsrs';

/** How many bytes a file is closed at: 8 MiB. */
const MAX_FILE_SIZE: number = 8 * 1024 * 1024;

/** How many seconds a file is kept: ten days. */
const MAX_ALIVE_TIME: number = 10 * 24 * 60 * 60;

/** What every record of the demo says, one at a level, in this order. */
const RECORDS: [LogLevel, string, string][] = [
  [LogLevel.Verbose, 'trace', 'the finest record there is'],
  [LogLevel.Debug, 'net', 'resolved 3 addresses for example.com'],
  [LogLevel.Info, 'startup', 'cold start in 412 ms'],
  [LogLevel.Warning, 'net', 'retrying after 1204 ms'],
  [LogLevel.Error, 'login', 'login failed: token expired'],
  [LogLevel.Fatal, 'login', 'giving up after 3 attempts'],
];

/**
 * The appender of the demo, and the directory it writes into.
 *
 * Two of them are never open at once: the module keeps one, and `openLogStore`
 * answers that one to every caller, because a prefix is one appender on the
 * native side and a second `Xlog.open` of the same prefix is one appender two
 * names hold.
 */
export class LogStore {
  /** Where the `.xlog` files are. The page shows it. */
  readonly directory: string;

  /** Why there is no appender, and `undefined` when there is one. */
  readonly failure: string | undefined;

  private xlog: Xlog | undefined = undefined;

  private constructor(directory: string, failure: string | undefined, xlog: Xlog | undefined) {
    this.directory = directory;
    this.failure = failure;
    this.xlog = xlog;
  }

  /** `Xlog.open` over the directory the app was given.
   *
   * It does not throw: `Xlog.open` does, when the appender refuses the
   * configuration, and an app that lets that reach the ability's `onCreate` is
   * an app that dies on a device whose storage is full. So the reason is kept
   * and the app runs without a log, which is what an app that cannot log has to
   * be able to do anyway. */
  static open(context: common.UIAbilityContext): LogStore {
    // `filesDir` is the directory the system gives this app and the one its
    // files belong in — the HarmonyOS answer to Android's `filesDir` and to
    // Apple's Application Support, and the only directory an app may write
    // without asking for a permission.
    const directory: string = `${context.filesDir}/xlog`;
    try {
      // Every field but `logDir` has the default the C++ gives it, so an app
      // names the four it cares about: the prefix the files start with, the
      // level the appender is opened at, the mode it writes in and how a closed
      // file is compressed. `.zlib` is what the C++ writes.
      const xlog: Xlog = Xlog.open({
        logDir: directory,
        // The mmap cache, beside the files. It is what a record sits in between
        // the call and the writer thread, and it is a file the appender needs to
        // find again, so it goes in the app's own directory and not in one the
        // system empties when space runs short.
        cacheDir: `${context.filesDir}/xlog-cache`,
        namePrefix: NAME_PREFIX,
        // Verbose so that all six records survive to be read back. An app that
        // ships sets `LogLevel.Info`, and `LogLevel.None` for a build that
        // writes nothing at all.
        level: LogLevel.Verbose,
        // `Async` hands the record to a writer thread and returns — the whole
        // point of the pipeline is that a log call does not block the thread
        // that made it.
        mode: AppenderMode.Async,
        compressMode: CompressMode.Zlib,
      });

      // Mirror every record to the console as well, so DevEco's HiLog shows
      // what went into the file. Off in a build that ships.
      xlog.consoleLogEnabled = true;
      // Close a file at 8 MiB and drop one at ten days. Both start at 0, which
      // is not the same 0 twice: a maximum size of 0 never splits a file, and a
      // lifetime of 0 is the C++'s own ten days.
      xlog.maxFileSizeBytes = MAX_FILE_SIZE;
      xlog.maxAliveTimeSeconds = MAX_ALIVE_TIME;

      return new LogStore(directory, undefined, xlog);
    } catch (error) {
      return new LogStore(directory, (error as Error).message, undefined);
    }
  }

  /** Whether there is an appender to write through: `false` when `Xlog.open`
   * threw, which is what a directory the app cannot write to comes to. */
  get isOpen(): boolean {
    return this.xlog !== undefined;
  }

  /** One record at every level, and the flush that leaves them on disk.
   *
   * The same six `demo/rust`, `demo/c`, `demo/android` and `demo/kmp` write, in
   * the same order, so that the five read side by side. */
  writeDemoRecords(): void {
    const xlog: Xlog | undefined = this.xlog;
    if (xlog === undefined) {
      return;
    }

    for (const record of RECORDS) {
      xlog.log(record[0], record[1], record[2]);
    }

    // A message that is expensive to build is worth asking about first: a
    // record the level drops still costs its caller the string.
    if (xlog.isLoggable(LogLevel.Debug)) {
      xlog.d('startup', `the appender is ${xlog.namePrefix}`);
    }

    // `flushNow` waits for the write, so every record above is on disk when
    // this returns. An app calls it before it reads the files or uploads
    // them.
    xlog.flushNow();
  }

  /** Closes the appender: drains what is left and drops it.
   *
   * Called from the ability's `onDestroy`, which is the point at which the
   * system may take the process away — and the last moment an async appender's
   * own writer thread is guaranteed to still be running. */
  close(): void {
    this.xlog?.close();
    this.xlog = undefined;
  }
}

/** The one `LogStore` of the process, and `undefined` until `openLogStore` runs. */
let current: LogStore | undefined = undefined;

/** Opens the demo's appender, or answers the one that is already open.
 *
 * Called from `EntryAbility.onCreate`, the one place `context.filesDir` is
 * reachable before the first page is built. */
export function openLogStore(context: common.UIAbilityContext): LogStore {
  if (current === undefined) {
    current = LogStore.open(context);
  }
  return current;
}

/** The appender the pages write through, or `undefined` before `onCreate`. */
export function logStore(): LogStore | undefined {
  return current;
}

/** Closes the appender the ability opened. */
export function closeLogStore(): void {
  current?.close();
  current = undefined;
}
