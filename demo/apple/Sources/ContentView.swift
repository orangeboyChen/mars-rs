// The view of the Apple demo: one button that writes the six records, and one
// line that says where they went.
//
// Everything it does with the port is one call on `LogStore`; the point of the
// file is the opposite one — that an app's screens need know nothing about the
// appender to log through it.

import SwiftUI

struct ContentView: View {
    /// The appender the `App` opened. `@ObservedObject` and not a second
    /// `LogStore()`: the appender is one per process, and a view that built its
    /// own would be writing through a second one over the same files.
    @ObservedObject var log: LogStore

    var body: some View {
        VStack(spacing: 24) {
            Text(log.status)
                .font(.footnote.monospaced())
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)

            Button("Write six records") {
                log.writeDemoRecords()
            }
            .buttonStyle(.borderedProminent)
            // Off when there is no appender: a button that writes nowhere is a
            // button that looks broken when it is tapped.
            .disabled(!log.isOpen)
        }
        .padding()
    }
}
