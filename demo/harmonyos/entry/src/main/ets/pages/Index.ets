// The page of the HarmonyOS demo: one button that writes the six records, and
// one line that says where they went.
//
// Everything it does with the port is one call on `LogStore`; the point of the
// file is the opposite one — that an app's screens need know nothing about the
// appender to log through it.

import { LogStore, logStore } from '../model/LogStore';

@Entry
@Component
struct Index {
  /** What the page shows: the directory being written into, or why nothing is
   * being written at all. */
  @State status: string = 'opening the appender';

  /** Whether there is an appender the button may write through. */
  @State canWrite: boolean = false;

  aboutToAppear(): void {
    // The ability opened it in `onCreate`, before this page existed.
    const log: LogStore | undefined = logStore();
    if (log === undefined) {
      this.status = 'no appender: the ability has not started';
      return;
    }
    if (!log.isOpen) {
      this.status = `no appender: ${log.failure ?? 'it was closed'}`;
      return;
    }
    this.status = `writing into ${log.directory}`;
    this.canWrite = true;
  }

  build() {
    Column({ space: 24 }) {
      Text(this.status)
        .fontSize(14)
        .fontFamily('monospace')
        .width('100%')
        .textAlign(TextAlign.Start)

      Button($r('app.string.write_records'))
        .enabled(this.canWrite)
        .onClick(() => {
          logStore()?.writeDemoRecords();
          this.status = 'six records written and flushed';
        })
    }
    .padding(24)
    .width('100%')
    .height('100%')
    .justifyContent(FlexAlign.Center)
  }
}
