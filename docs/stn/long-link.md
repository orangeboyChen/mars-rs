# The long link

A long link is a connection and not a request, and it is the app's to keep up:
it carries your tasks, what the server pushes down, and the noop that tells the
middle boxes the connection is still wanted.

| what | Rust | Swift | Android, KMP | C |
|---|---|---|---|---|
| where it connects | `set_longlink_svr_addr` | `setLongLinkServerAddress` | `setLonglinkSvrAddr` | `mars_stn_set_longlink_svr_addr` |
| where the short link goes | `set_shortlink_svr_addr` | `setShortLinkServerAddress` | `setShortlinkSvrAddr` | `mars_stn_set_shortlink_svr_addr` |
| force a connect | `make_sure_long_link_connected` | `makeSureLongLinkConnected` | `makesureLongLinkConnected` | `mars_stn_makesure_longlink_connected` |
| force a connect on one you named | `make_sure_long_link_connected` | `makeSureLongLinkConnected(name:)` | `makesureLongLinkConnectedExt` | `mars_stn_makesure_longlink_connected_ext` |
| is it up | `is_default_long_link_connected` | `isLongLinkConnected` | `longLinkIsConnected` | `mars_stn_longlink_is_connected` |
| is one you named up | `is_long_link_connected` | `isLongLinkConnected(name:)` | `longLinkIsConnectedExt` | `mars_stn_longlink_is_connected_ext` |
| stop using it | `disable_long_link` | `disableLongLink` | `disableLongLink` | `mars_stn_disable_longlink` |
| the noop's task id | `Task::NOOP_TASK_ID` | `noopTaskID` | `noopTaskID` | `mars_stn_noop_task_id` |
| the keepalive | `keep_signalling` / `stop_signalling` | `keepSignalling` / `stopSignalling` | `keepSignalling` / `stopSignalling` | `mars_stn_keep_signalling` / `mars_stn_stop_signalling` |
| send a noop now | — | `triggerNooping` | `trigNooping` | `mars_stn_trig_nooping` |
| throw it away and start over | `reset` / `reset_with_encoder` | `reset` / `resetAndInitEncoderVersion` | `reset` / `resetAndInitEncoderVersion` | `mars_stn_reset` / `mars_stn_reset_and_init_encoder_version` |

`makesureLongLinkConnected()` answers nothing on Android and in the shared
Kotlin — the C ABI's symbol answers 1 or 0 and the JNI one answers `void` — so a
caller who wants to know reads `Question.linkStatus` instead.

## When the network or the screen changes

STN is told, in the spelling of the platform:

| where | how |
|---|---|
| Android | `BaseEvent.onNetworkChange()`, and `BaseEvent.onForeground(true / false)` when the app comes back or goes away |
| everywhere else | `reset` — and the two addresses above, when where it connects has changed |

A short link needs no telling: a task goes out on whichever link is up, and
`Task::new` asks for both.

"Up" is one state and not "not down": a link that is still connecting answers
`false`. And stopping is a one-way door — `disable_long_link` drops the long link
for good, and only `reset` makes a core that uses one again.

## Where to go next

- [The questions](/stn/callbacks) — the eighteen the app answers while a task
  runs, and the boot that installs them on Android.
- [The task](/stn/tasks) — what a task is made of, and how one ends.
