# 长连接

长连接是一条连接，不是一个请求，而且它由 App 来保持：它承载你的任务、服务器推下来
的东西，以及告诉中间那些盒子这条连接还要用的 noop。

| 什么 | Rust | Swift | Android、KMP | C |
|---|---|---|---|---|
| 它连到哪里 | `set_longlink_svr_addr` | `setLongLinkServerAddress` | `setLonglinkSvrAddr` | `mars_stn_set_longlink_svr_addr` |
| 短连接去哪里 | `set_shortlink_svr_addr` | `setShortLinkServerAddress` | `setShortlinkSvrAddr` | `mars_stn_set_shortlink_svr_addr` |
| 强制连一次 | `make_sure_long_link_connected` | `makeSureLongLinkConnected` | `makesureLongLinkConnected` | `mars_stn_makesure_longlink_connected` |
| 强制连一次你命名的那条 | `make_sure_long_link_connected` | `makeSureLongLinkConnected(name:)` | `makesureLongLinkConnectedExt` | `mars_stn_makesure_longlink_connected_ext` |
| 它连上了吗 | `is_default_long_link_connected` | `isLongLinkConnected` | `longLinkIsConnected` | `mars_stn_longlink_is_connected` |
| 你命名的那条连上了吗 | `is_long_link_connected` | `isLongLinkConnected(name:)` | `longLinkIsConnectedExt` | `mars_stn_longlink_is_connected_ext` |
| 不再走长连接 | `disable_long_link` | `disableLongLink` | `disableLongLink` | `mars_stn_disable_longlink` |
| noop 的任务 id | `Task::NOOP_TASK_ID` | `noopTaskID` | `noopTaskID` | `mars_stn_noop_task_id` |
| 保活 | `keep_signalling` / `stop_signalling` | `keepSignalling` / `stopSignalling` | `keepSignalling` / `stopSignalling` | `mars_stn_keep_signalling` / `mars_stn_stop_signalling` |
| 现在就发一个 noop | — | `triggerNooping` | `trigNooping` | `mars_stn_trig_nooping` |
| 丢掉重来 | `reset` / `reset_with_encoder` | `reset` / `resetAndInitEncoderVersion` | `reset` / `resetAndInitEncoderVersion` | `mars_stn_reset` / `mars_stn_reset_and_init_encoder_version` |

`makesureLongLinkConnected()` 在 Android 和共享 Kotlin 上什么也不回答 —— C ABI 的
那个符号回答 1 或 0，JNI 那个回答 `void` —— 想知道的调用方读 `Question.linkStatus`。

## 网络或屏幕变了的时候

按平台的写法告诉 STN：

| 哪里 | 怎么告诉 |
|---|---|
| Android | `BaseEvent.onNetworkChange()`，以及 App 回来或离开时 `BaseEvent.onForeground(true / false)` |
| 其余每个平台 | `reset` —— 连的地方变了的话，再加上上面那两个地址 |

短连接不需要告诉：一个任务会走第一条已经起来的链路，`Task::new` 两条都要。

“连上了”是一个状态，不是“没断开”：还在连的那一条回答 `false`。关掉也是单向的
—— `disable_long_link` 把长连接彻底放下，只有 `reset` 会造出一个重新用它的 core。

## 接着看

- [那些问题](/zh/stn/callbacks) —— App 在一个任务跑着的时候回答的那十八个，以及
  Android 上装它们的那次启动。
- [任务](/zh/stn/tasks) —— 一个任务由什么构成，以及它是怎么结束的。
