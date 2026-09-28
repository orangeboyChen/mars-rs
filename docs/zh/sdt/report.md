# 报告

每一项检查写一个 `CheckResultProfile`，报告就是它们变成的 JSON ——
`{"details":[ … ]}`，一项检查一个对象：

```json
{
  "details": [
    {
      "detectType": 1, "errorCode": 0, "networkType": 1,
      "detectIP": "1.2.3.4", "port": 80, "conntime": 30, "rtt": 42,
      "rttStr": "42ms", "httpStatusCode": 0, "pingCheckCount": 0,
      "pingLossRate": "", "dnsDomain": "example.com", "localDns": "",
      "dnsIP1": "1.2.3.4", "dnsIP2": ""
    }
  ]
}
```

| 什么 | 那个字段 | 它是什么 |
|---|---|---|
| 哪一项检查 | `detectType` | [检查项](/zh/sdt/checks)那页的那些整数之一 |
| 探测跑得怎么样 | `errorCode` | `0` 及以上是跑成了的 |
| 它跑在哪个网络上 | `networkType` | 你交给这一趟的那个 `comm::getNetInfo()` |
| 探测了什么 | `detectIP`、`port`、`dnsDomain` | 那个 host，tcp 检查还有端口 |
| 那些耗时 | `conntime`、`rtt`、`rttStr` | 毫秒；`rttStr` 是 `rtt` 的字符串形式 |
| 那次 HTTP 检查 | `httpStatusCode` | net-check CGI 回答的 |
| 那个 ping | `pingCheckCount`、`pingLossRate` | 发了多少个，以及丢了多大比例 |
| 那个 dns | `localDns`、`dnsIP1`、`dnsIP2` | 那个解析器，以及头两个地址 |

## 取它

| 什么 | Rust | Swift | 共享 Kotlin、Android | C |
|---|---|---|---|---|
| 报告 | `report_json(&results)` | `MarsSdt.takeReport()` | `SdtLogic.takeReport()` | `mars_sdt_take_report(buf, len)` |
| 改用回调 | — | — | `SdtLogic.setCallBack { … }` | — |

两条路取的是同一份报告，所以 App 走一条就够：在 Android 和共享 Kotlin 上，一趟会把
报告交给回调*并且*留着给之后的 `takeReport()`，所以两个都用的 App 会把同一次诊断送
两次。取走就清空了 —— 下一次调用报的是这之后发生的事。

在 Android 和共享 Kotlin 上报告是一个 `String?`，所以没有 buffer 要估大小。Swift
的 `takeReport()` 从 4 KB 起、翻倍到 1 MB，只有没东西可拿的时候才回答 `nil`。
`mars_sdt_take_report` 在报告塞不进 buffer 时回答 `MARS_SDT_ERR_NO_SPACE` —— 而且
**结果留着**，所以拿一个更大的 buffer 再问一次的调用方拿到的是那次诊断，而不是一份
空的。

## 取消，以及从头再来

| 什么 | Rust | Swift | Kotlin | C |
|---|---|---|---|---|
| 停掉这一趟 | `cancel_active_check` / `cancel_handle` | `cancelActiveCheck` | `cancelActiveCheck` | `mars_sdt_cancel_active_check` |
| 有一趟在跑吗 | `is_checking` | `isChecking` | `isChecking` | `mars_sdt_is_checking` |
| 全丢掉 | `SdtCore::reset` | `reset` | `reset` | `mars_sdt_reset` |

在 Rust 里一趟会独占借用这个 logic，所以 `run_checks` 还在栈上的时候调不了
`cancel_active_check`：先用 `cancel_handle()` 取一个 `CancelHandle`，交给要停这一趟
的那一方 —— 那个探测闭包，socket 在它手里，只有它放得开。

其余每个平台取消都不用 handle：它设的是这一趟会读的那个标志，不去拿这一趟握着的
锁，所以探测还在被问的时候，它能从另一个线程落下来 —— 这本来就是取消唯一有意义的
时刻。它做的是让计划里剩下的部分不再跑起来；已经出去的探测不会被打断。

还有一点，一次只跑一趟：诊断是一个进程级的值，所以第二次 `runChecks` 会等第一次跑完，
而不是跟它并排跑 —— 在 Android 和共享 Kotlin 上，第二次调用要到第一次结束才返回。

## 接着看

- [探测](/zh/sdt/probes) —— 那四项检查，以及每一项要 App 给什么。
- [快速开始](/zh/sdt/getting-started) —— 依赖和一整趟诊断。
