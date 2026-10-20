# The task

A task is one struct, and every field has the default mars gives it. The ones an
app sets:

| what it is | Rust | Swift | Android, KMP | C |
|---|---|---|---|---|
| the id you stop it by | `taskid` | `taskID` | `taskID` | `taskid` |
| what the server calls it | `cmdid` | `cmdid` | `cmdID` | `cmdid` |
| the path it goes to | `cgi` | `cgi` | `cgi` | `cgi` |
| which link(s) | `channel_select` | `channelSelect` | `channelSelect` | `channel_select` |
| the hosts to try | `shortlink_host_list`, `longlink_host_list` | `shortLinkHosts`, `longLinkHosts` | `shortLinkHostList` | `shortlink_host_list` |
| how long, in ms | `total_timeout` | `totalTimeout` | `totalTimeout` | `total_timeout` |
| how many retries | `retry_count` | `retryCount` | `retryCount` | `retry_count` |
| headers | `headers` | `headers` | `headers` | `headers` / `header_count` |

`Task::new` in Rust — `StnTask(channelSelect:)` in Swift, `Task()` in the shared
Kotlin — draws the shape for you, so what an app names is the path, the hosts and
the timeout. `gen_task_id()` / `MarsStn.generateTaskID()` /
`StnLogic.genTaskID()` is there when the id is wanted before the task.

The channel is the one field an app names first: a task with `channel_select`
`0` goes nowhere, because that is what a net core refuses — and of the three
above only Rust's `Task::new` fills it in, with `CHANNEL_BOTH`. `StnTask`
starts with no channel and `Task()` in the shared Kotlin with `0`, which is
what the C++'s `Task::Task()` gives and what its Java makes the app set, so on
those the app names it: `MarsStn.Channel.both`, or `Task.E_BOTH`, or on Android
the first argument of `Task(channelselect, cmdid, cgi, hostList)`.

`need_authed` is `true` in Rust, in Swift, and in that four-argument Android
`Task` — the C++ project's own Java answers `true` there too. A `Task()` an app
builds with no arguments leaves it `false`.

## Starting, stopping, and whether one is there

| what | Rust | Swift | Android, KMP | C |
|---|---|---|---|---|
| start it | `stn.start_task(task)` | `MarsStn.start(task)` | `StnLogic.startTask(task)` | `mars_stn_start_task(&task)` |
| start it and await the answer | `stn.send(task, body)` | — | — | — |
| stop it | `stn.stop_task(id)` | `MarsStn.stop(taskID:)` | `StnLogic.stopTask(id)` | `mars_stn_stop_task(id)` |
| is it still there | `stn.has_task(id)` | `MarsStn.hasTask(id)` | `StnLogic.hasTask(id)` | `mars_stn_has_task(id)` |

`start_task` returns at once: the task runs on the queue and not on the calling
thread. `has_task` answers `true` for a task that is going nowhere, too — one
that was started and never drained.

`send` is Rust's, and it is the one to take for a task whose answer you want: it
starts the task and hands back a `Sent`, a future whose output is the answer or
the failure of the run. `body` is the bytes the task sends — what `req2buf`
would have been asked for — and `send_at(now, task, body)` is the same call with
the clock reading handed in. In Rust `start_task` is deprecated in favour of
`send`; `start_task_at` is not, and neither is the `start` of any other
platform, which is the upstream call for a task nobody awaits.

## How a task ends

The app hears the end as a question — `onTaskEnd` in every spelling — carrying
two numbers and a profile:

- **an error type** — *where* it failed: `ErrCmdType` in Rust, `errType` on the
  others. `Ok` (0), or `Dns`, `Socket`, `Http`, `Server`, `Local`, `Canceld` …
- **an error code** — *what* went wrong, and it is negative: `-500` a first packet
  that never came, `-501` a gap between packets, `-502` a read or a write that
  timed out, `-503` the task as a whole, `-10086` the network changed under it.
  Android names these on `StnLogic` (`FIRSTPKGTIMEOUT`, `TASKTIMEOUT`, …).
- **a profile** — the timings of the run: when the DNS started, when the connect
  finished, the rtt. `CgiProfile` in Rust and Kotlin, `StnQuestion.CgiProfile` in
  Swift, `MarsStnCgiProfile` in C.

`buf2Resp` answers with a **fail handle** as well as a code, which is how the app
tells STN what to do about a bad answer rather than only that it was one:
`Normal`, `RetryAllTasks`, `SessionTimeout`, `TaskEnd`, `TaskTimeout`.

### Awaiting it instead, in Rust

In Rust a task can be awaited and not only listened for. What `send` hands back
is a `Sent`, and its `Output` is:

- **`Ok(Answer)`** — `body`, the bytes the server answered with, which is what
  `buf2Resp` would have been handed; and `profile`, the timings of the connect.
- **`Err(Failure)`** — why there is no answer. `NotCreated` is no net core yet:
  `create()` has not been through. `Refused` is a released core, which starts
  nothing and reports nothing. `Ended { err_type, err_code, profile }` is where
  it failed and with what code — the same two numbers `on_task_end` carries —
  and it is also how a task the core refused on its way in is reported.

A `Sent` borrows nothing, so it is awaited wherever the app's own executor puts
it, and the lock the logic sits behind is let go before the `.await`: holding it
across the `.await` deadlocks the pass that would answer it. A `Sent` nothing
drains stays `Pending`, the way a task that is started and never drained stays in
its queue.

The questions do not stop: every one of them is still asked of the app while an
awaited task runs, `on_task_end` among them, so awaiting is a value the app takes
and not a question the app stops being asked. What falls through to the app is
the two a request and its answer would have been asked for — `req2buf` and
`buf2resp` — for a task nobody is awaiting. See
[the questions](/stn/callbacks).

## What a task runs on

Nothing drains the queue for you. `run_pending` / `due_delay` — in every spelling
on [the getting started page](/stn/getting-started) — is what moves a task out of
it, and no thread is started unless an app asks for one: the loop is the app's,
and a task that is started and never drained sits in its queue until the process
ends.

In Rust one call is enough: `Driver::spawn(stn)` starts a thread of this crate's
that does what the host loop does — `run_pending()` at the end of every 20 ms
slice, and a sleep of that slice between two passes, which is *not* the delay
`due_delay()` answers: that one is how long a host that is woken by its own
sockets may wait, and a thread of this crate's is woken by nothing — and the
thread is joined when the `Driver` is dropped, unless the drop is itself holding
the logic the thread wants, in which case the stop flag ends it.
The logic is shared and not moved, so an app keeps its own `Arc` and goes on
starting tasks through it. A host that already has a `run_pending` loop keeps it
and takes no `Driver`: a pass that ends a task wakes whoever awaited it, so a
`Driver` and a host's own loop work together.

`due_delay()` is how long the host may wait until the next pass is due, in
milliseconds: `0` is a pass that is already due, which a follow-up waiting in the
queue is. It is a duration and not a time of day, which is what a caller across
an ABI needs — a tick is measured from an origin only this process can read, so
the host is told the wait instead. What it is not is a promise: a loop that calls
`run_pending` in a tight spin works too, and burns a core doing it.

On Android and in the shared Kotlin the loop is the one place an app sees the two
bridges disagree about a shape: `StnLogic.dueTime()` answers `Long?` — `null`
when there is nothing to wait for — because the JNI bridge answers `-1` and the C
ABI its own `MARS_STN_ERR_NO_DUE`.
