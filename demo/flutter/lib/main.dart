// The Flutter demo of mars-rs: one screen, one button, six records into a
// `.xlog` file.
//
// The whole app is this file. What it does is what every demo in `demo/` does —
// open an appender, write one record at every level, flush — under the one name
// the port gives that call on every platform: `Xlog.open(config)`. What is
// Flutter's is the two `await`s: a method channel is a round trip to the
// platform, so opening an appender and draining it are futures, and a write is
// not.
//
// Run it with `flutter run`, from the directory `README.md` says to create.

import 'package:flutter/material.dart';
import 'package:marsrs_xlog/marsrs_xlog.dart';
import 'package:path_provider/path_provider.dart';

/// The prefix of every file: `marsrs_YYYYMMDD.xlog`.
const String namePrefix = 'marsrs';

/// How many bytes a file is closed at: 8 MiB.
const int maxFileSize = 8 * 1024 * 1024;

/// How many seconds a file is kept: ten days.
const int maxAliveTime = 10 * 24 * 60 * 60;

/// Opens the appender, before the first frame is drawn.
///
/// It is a `main` that returns a future and not the `void main()` a Flutter
/// template writes, because `Xlog.open` is one: the appender lives on the other
/// side of a method channel, and a widget that had to wait for it would have to
/// rebuild when it arrived.
///
/// An appender that would not open is `null` here and not a crash: the plugin
/// answers a `PlatformException` when the platform refuses the configuration,
/// and an app that lets that reach `main` is an app that dies on a device whose
/// storage is full.
Future<Xlog?> openXlog() async {
  // `ensureInitialized` is what makes a channel callable before `runApp`.
  WidgetsFlutterBinding.ensureInitialized();

  // The directory the platform gives this app: Application Support, which is
  // the one a log belongs in — it is the app's own, it is backed up, and no one
  // browses it. `getTemporaryDirectory()` is the one the OS empties when space
  // runs short, which is exactly when a log matters.
  final directory = await getApplicationSupportDirectory();
  try {
    final xlog = await Xlog.open(
      XlogConfig(
        // The one field with no default — the appender is not opened without
        // it. The rest are the C++'s own: `null` puts the mmap cache beside the
        // files, and `.zlib` is what the C++ writes.
        logDir: '${directory.path}/xlog',
        namePrefix: namePrefix,
        // Verbose so that all six records survive to be read back. An app that
        // ships sets `LogLevel.info`, and `LogLevel.none` for a build that
        // writes nothing at all.
        level: LogLevel.verbose,
        // `.async` hands the record to a writer thread and returns — the whole
        // point of the pipeline is that a log call does not block the isolate
        // that made it.
        mode: AppenderMode.async,
      ),
    );
    // Mirror every record to the console as well, so `flutter run` shows what
    // went into the file. Off in a build that ships.
    xlog.consoleLogEnabled = true;
    // Close a file at 8 MiB and drop one at ten days. Both start at 0, which is
    // not the same 0 twice: a maximum size of 0 never splits a file, and a
    // lifetime of 0 is the C++'s own ten days.
    xlog.maxFileSizeBytes = maxFileSize;
    xlog.maxAliveTimeSeconds = maxAliveTime;
    return xlog;
  } catch (error) {
    // `debugPrint` and not `print`: a long message is dropped by the latter on
    // Android, and the reason an appender would not open is a long one.
    debugPrint('marsrs-xlog opened no appender: $error');
    return null;
  }
}

Future<void> main() async {
  runApp(MarsRSDemoApp(await openXlog()));
}

/// The app: the appender it opened, and one screen.
class MarsRSDemoApp extends StatelessWidget {
  const MarsRSDemoApp(this.xlog, {super.key});

  /// The appender every screen below writes through, or `null` when none was
  /// opened.
  final Xlog? xlog;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'MarsRS Demo',
      theme: ThemeData(useMaterial3: true),
      home: LogPage(xlog: xlog),
    );
  }
}

/// The one screen: what the appender is writing into, and a button that writes
/// six records through it.
class LogPage extends StatefulWidget {
  const LogPage({required this.xlog, super.key});

  /// The appender, or `null` when none was opened.
  final Xlog? xlog;

  @override
  State<LogPage> createState() => _LogPageState();
}

class _LogPageState extends State<LogPage> with WidgetsBindingObserver {
  /// What the screen says: what the last write did.
  String status = 'open';

  @override
  void initState() {
    super.initState();
    // The last moment the app is told anything before the OS can take the
    // process away is the lifecycle change below, and with an async appender
    // it is the one an app has to answer with a drain. On Android the AAR does
    // this on its own, from a `Context` an app handed it; Dart gets no
    // `Context`, so the observer is the app's.
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state != AppLifecycleState.paused && state != AppLifecycleState.hidden) {
      return;
    }
    widget.xlog?.flush();
  }

  /// Writes one record at every level, and waits for all of them to land.
  ///
  /// The six are the same six `demo/rust`, `demo/c`, `demo/android` and
  /// `demo/kmp` write, in the same order, so that the five read side by side.
  Future<void> writeDemoRecords() async {
    final xlog = widget.xlog;
    if (xlog == null) {
      setState(() => status = 'no appender was opened');
      return;
    }

    // The writes are not awaited: a write hands the record to the appender and
    // returns, and the call that waits for the six of them is the flush.
    xlog.v('trace', 'the finest record there is');
    xlog.d('net', 'resolved 3 addresses for example.com');
    xlog.i('startup', 'cold start in 412 ms');
    xlog.w('net', 'retrying after 1204 ms');
    xlog.e('login', 'login failed: token expired');
    xlog.f('login', 'giving up after 3 attempts');

    // A message that is expensive to build is worth asking about first — and in
    // Dart this is the one check that *is* a future, because it asks the
    // appender itself rather than a field this side holds.
    if (await xlog.isLoggable(LogLevel.debug)) {
      xlog.d('startup', 'the appender is ${xlog.namePrefix}');
    }

    // `flush` answers a future that is settled once the platform side has
    // drained, so every record above is on disk when the `await` returns. An
    // app calls it before it reads the files or uploads them.
    //
    // Dart is the one platform with no `flushNow` to call instead, and it is
    // deliberate: a method channel is a message and an answer, and there is
    // no blocking on this side of one. `requestFlush()` asks for the same drain
    // and returns at once — it never says when the drain is over, so it is not
    // the one to call before the files are read or uploaded.
    await xlog.flush();
    setState(() => status = 'six records written and flushed');
  }

  @override
  Widget build(BuildContext context) {
    // `xlog` once, so that the null check the button needs and the one the line
    // of text needs are the same check.
    final xlog = widget.xlog;
    final where = xlog == null
        ? 'no appender was opened'
        : 'writing into ${xlog.namePrefix}_<YYYYMMDD>.xlog';
    return Scaffold(
      appBar: AppBar(title: const Text('MarsRS Demo')),
      body: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(where),
            const SizedBox(height: 24),
            FilledButton(
              // Off when there is no appender: a button that writes nowhere is
              // a button that looks broken when it is tapped.
              onPressed: xlog == null ? null : writeDemoRecords,
              child: const Text('Write six records'),
            ),
            const SizedBox(height: 24),
            Text(status),
          ],
        ),
      ),
    );
  }
}
