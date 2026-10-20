# 长连接

长连接是一条连接，不是一个请求，而且要 App 自己保持：它承载你的任务、服务器推下来
的东西，以及 noop，它告诉中间那些盒子这条连接还要用。

| 什么 | Rust | Swift | Android、KMP | C |
|---|---|---|---|---|
| 它连到哪里 | `set_longlink_svr_addr` | `setLongLinkServerAddress` | `setLonglinkSvrAddr` | `mars_stn_set_longlink_svr_addr` |
| 短连接去哪里 | `set_shortlink_svr_addr` | `setShortLinkServerAddress` | `setShortlinkSvrAddr` | `mars_stn_set_shortlink_svr_addr` |
| 强制连一次 | `make_sure_default_long_link_connected` | `makeSureLongLinkConnected` | `makesureLongLinkConnected` | `mars_stn_makesure_longlink_connected` |
| 强制连一次你命名的那条 | `make_sure_long_link_connected` | `makeSureLongLinkConnected(name:)` | `makesureLongLinkConnectedExt` | `mars_stn_makesure_longlink_connected_ext` |
| 再开一条有名的 | `create_long_link` | `createLongLink` | `createLonglink` | `mars_stn_create_longlink` |
| 丢掉那条有名的 | `destroy_long_link` | `destroyLongLink` | `destroyLonglink` | `mars_stn_destroy_longlink` |
| 哪一条算主连接 | `mark_main_longlink` | `markMainLongLink` | `markMainLonglink` | `mars_stn_mark_main_longlink` |
| 它连上了吗 | `is_default_long_link_connected` | `isLongLinkConnected` | `longLinkIsConnected` | `mars_stn_longlink_is_connected` |
| 你命名的那条连上了吗 | `is_long_link_connected` | `isLongLinkConnected(name:)` | `longLinkIsConnectedExt` | `mars_stn_longlink_is_connected_ext` |
| 不再走长连接 | `disable_long_link` | `disableLongLink` | `disableLongLink` | `mars_stn_disable_longlink` |
| noop 的任务 id | `Task::NOOP_TASK_ID` | `noopTaskID` | `noopTaskID` | `mars_stn_noop_task_id` |
| 保活 | `keep_signalling` / `stop_signalling` | `keepSignalling` / `stopSignalling` | `keepSignalling` / `stopSignalling` | `mars_stn_keep_signalling` / `mars_stn_stop_signalling` |
| 现在就发一个 noop | — | `triggerNooping` | `trigNooping` | `mars_stn_trig_nooping` |
| 丢掉重来 | `reset` / `reset_with_encoder` | `reset` / `resetAndInitEncoderVersion` | `reset` / `resetAndInitEncoderVersion` | `mars_stn_reset` / `mars_stn_reset_and_init_encoder_version` |

上面这些调用的都是默认那条长连接。要再开一条，应用得给它一个名字 —— `create_long_link`
加一个 `LonglinkConfig`：名字、host 列表、要不要自己重连、要不要向应用报状态。之后
就按这个名字找它：`destroy_long_link` 把它丢掉、并让正在它上面跑的任务全部失败，
`mark_main_longlink` 让它成为给应用报状态的那一条。除 Rust 之外的每个平台上，`group`
留空、`link_type` 给 `0`，拿到的就是长连接的默认值；Rust 里 `LonglinkConfig::new`
已经把它们填好了 —— `group` 是 `default-group`，`link_type` 是 `Task::CHANNEL_LONG`。

`makesureLongLinkConnected()` 在 Android 和共享 Kotlin 上什么也不回答 —— C ABI 的
那个符号回答 1 或 0，JNI 那个回答 `void` —— 想知道的调用方读 `Question.linkStatus`。

## 网络或屏幕变了的时候

按平台的写法告诉 STN：

| 哪里 | 怎么告诉 |
|---|---|
| Android | `BaseEvent.onNetworkChange()`，以及 App 回来或离开时 `BaseEvent.onForeground(true / false)` |
| Rust | `stn.on_network_change { … }` —— 闭包里是宿主自己的变更，它比 STN 的先跑 |
| Swift | `MarsStn.onNetworkChange()`，以及 `MarsStn.onForeground(true / false)` |
| Kotlin Multiplatform | `BaseEvent.onNetworkChange()`，以及 `BaseEvent.onForeground(true / false)` —— 就是 Android 那两个名字 |
| C ABI，以及经过它的 HarmonyOS | `mars_stn_on_network_change()`，以及 `mars_stn_on_foreground(1 / 0)` |

一次网络变化做的事不止是记一笔：STN 会把每条长连接拆掉、重新去连 —— 立刻，或者
等它自己的 monitor 走到下一个间隔 —— 走在那条连接上的任务会先取消、再发起一次，
这样它们才会落到一条已经起来的链路上。

屏幕是 App 最容易忘的那一半，也是没有别的调用会替它说的那一半：App 不说自己到了
前台，STN 就当它在后台 —— 而一个任务要唤醒一条断开的长连接，只有 App 在前台、
而且是最近一刻钟之内到的前台才行。在后台待满十分钟，App 就变成不活跃：抗雪崩
检查和授时同步读的就是这个。C++ 用自己的一个 alarm 数这十分钟；自己驱动
`run_pending` 的宿主在那里数。

Rust 回答屏幕的方式和它回答网络的方式一样：`stn.set_is_foreground { … }` 是
STN 问的时候取的一次读数，所以宿主没有要告诉它的东西。

短连接不需要告诉：一个任务会走第一条已经起来的链路，`Task::new` 两条都要。

“连上了”是一个状态，不是“没断开”：还在连的那一条回答 `false`。关掉也是单向的
—— `disable_long_link` 把长连接彻底放下，只有 `reset` 会造出一个重新用它的 core。

## 接着看

- [那些问题](/zh/stn/callbacks) —— App 在一个任务跑着的时候回答的那十八个，以及
  Android 上装它们的那次启动。
- [任务](/zh/stn/tasks) —— 一个任务由什么构成，以及它是怎么结束的。
