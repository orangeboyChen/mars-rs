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
| name one of your own | `create_long_link` | `createLongLink` | `createLonglink` | `mars_stn_create_longlink` |
| throw one you named away | `destroy_long_link` | `destroyLongLink` | `destroyLonglink` | `mars_stn_destroy_longlink` |
| which one is the main one | `mark_main_longlink` | `markMainLongLink` | `markMainLonglink` | `mars_stn_mark_main_longlink` |
| is it up | `is_default_long_link_connected` | `isLongLinkConnected` | `longLinkIsConnected` | `mars_stn_longlink_is_connected` |
| is one you named up | `is_long_link_connected` | `isLongLinkConnected(name:)` | `longLinkIsConnectedExt` | `mars_stn_longlink_is_connected_ext` |
| stop using it | `disable_long_link` | `disableLongLink` | `disableLongLink` | `mars_stn_disable_longlink` |
| the noop's task id | `Task::NOOP_TASK_ID` | `noopTaskID` | `noopTaskID` | `mars_stn_noop_task_id` |
| the keepalive | `keep_signalling` / `stop_signalling` | `keepSignalling` / `stopSignalling` | `keepSignalling` / `stopSignalling` | `mars_stn_keep_signalling` / `mars_stn_stop_signalling` |
| send a noop now | — | `triggerNooping` | `trigNooping` | `mars_stn_trig_nooping` |
| throw it away and start over | `reset` / `reset_with_encoder` | `reset` / `resetAndInitEncoderVersion` | `reset` / `resetAndInitEncoderVersion` | `mars_stn_reset` / `mars_stn_reset_and_init_encoder_version` |

Every call above reaches the default long link. An app that keeps a second one
gives it a name — `create_long_link` and a `LonglinkConfig` of the name, the
hosts, whether it reconnects on its own and whether it is the link whose status
the app hears about — and reaches it by that name from then on:
`destroy_long_link` drops it and fails whatever was going out on it, and
`mark_main_longlink` makes it the one the app is told about. A config that leaves
`group` empty and `link_type` at `0` gets the long-link defaults.

`makesureLongLinkConnected()` answers nothing on Android and in the shared
Kotlin — the C ABI's symbol answers 1 or 0 and the JNI one answers `void` — so a
caller who wants to know reads `Question.linkStatus` instead.

## When the network or the screen changes

STN is told, in the spelling of the platform:

| where | how |
|---|---|
| Android | `BaseEvent.onNetworkChange()`, and `BaseEvent.onForeground(true / false)` when the app comes back or goes away |
| Rust | `stn.on_network_change { … }` — the closure is the host's own change, and it runs before STN's |
| everywhere else | `reset` — and the two addresses above, when where it connects has changed |

What a network change does is more than note it: every long link is taken down
and asked for again — at once, or after the interval its own monitor is on — and
the tasks that were out on one are cancelled and started again, which is what
puts them on a link that is up. An app on the C ABI, on HarmonyOS, in Swift or in
a shared Kotlin module has no call for it, so there `reset` is the whole of what
a change can ask for.

A short link needs no telling: a task goes out on whichever link is up, and
`Task::new` asks for both.

"Up" is one state and not "not down": a link that is still connecting answers
`false`. And stopping is a one-way door — `disable_long_link` drops the long link
for good, and only `reset` makes a core that uses one again.

## Where to go next

- [The questions](/stn/callbacks) — the eighteen the app answers while a task
  runs, and the boot that installs them on Android.
- [The task](/stn/tasks) — what a task is made of, and how one ends.
