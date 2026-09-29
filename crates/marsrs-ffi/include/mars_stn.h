/* Tencent is pleased to support the open source community by making Mars available.
 * Copyright (C) 2016 THL A29 Limited, a Tencent company. All rights reserved.
 *
 * Licensed under the MIT License (the "License"); you may not use this file except in
 * compliance with the License. You may obtain a copy of the License at
 * http://opensource.org/licenses/MIT
 *
 * Unless required by applicable law or agreed to in writing, software distributed under the License is
 * distributed on an "AS IS" basis, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND,
 * either express or implied. See the License for the specific language governing permissions and
 * limitations under the License.
 */

/*
 * mars_stn.h — C ABI of the Rust task pipeline (`marsrs-ffi`, the `stn` feature).
 *
 * This header mirrors the surface of
 *
 *     mars/stn/stn.h                (StnLogic: StartTask / StopTask / RedoTasks ...)
 *     mars/stn/stn_logic.cc         (the `StnManager` the Java2C calls reach)
 *     mars/stn/src/net_core.cc      (what runs a task: the queues, the links)
 *
 * and is the C equivalent of the JNI bridge of `mars/stn`, its
 * `*_Java2C.cc` pair.
 *
 * The one thing a caller supplies is the app: eighteen questions STN asks while
 * it runs a task — is the caller logged in, what does this task send, how is the
 * answer read, what is to be reported. The C++ asks them as eighteen virtuals of
 * a `Callback` the app inherits; here they are ONE function pointer,
 * `MarsStnAsk`, handed one tagged `MarsStnQuestion` and answering one tagged
 * `MarsStnAnswer`. Answer with `MarsStnAnswerNothing` — or hand no `ask` at all
 * — and STN takes its own answers, which are the ones a host with no app gets.
 *
 * The header is hand-written and checked in next to the crate so that a C/C++
 * or Swift caller can include it without running cbindgen. It is kept in sync
 * with `src/stn.rs` by `tests/stn_header_sync.rs`.
 *
 * Threading: every symbol may be called from any thread, but the task pipeline
 * is one process-wide value — the counterpart of the C++'s singleton — and the
 * app is asked while it is held. An `ask` must therefore not call another
 * `mars_stn_*`: everything a question carries is in the question itself.
 *
 * Panics: no Rust panic ever crosses this boundary. Every entry point is
 * wrapped in `catch_unwind`; a panic is reported as `MARS_STN_ERR_PANIC` (or
 * silently swallowed for the `void` symbols).
 */

#ifndef MARS_STN_H_
#define MARS_STN_H_

/* The symbols this header declares are the ones `#[no_mangle]` exports: plain
 * C names, and a C++ translation unit that includes this and calls
 * `mars_stn_start_task()` asks its linker for a mangled one that does not
 * exist. The guard says what the library already says for itself. */
#ifdef __cplusplus
extern "C" {
#endif

/* --- return codes ------------------------------------------------------- */

#define MARS_STN_OK 0
#define MARS_STN_ERR_PANIC (-1)    /* a Rust panic was caught at the boundary */
#define MARS_STN_ERR_NULL_TASK (-2) /* `mars_stn_start_task` got no task      */
#define MARS_STN_ERR_REFUSED (-3)  /* the queues would not take the task      */
#define MARS_STN_ERR_NO_DUE (-4)   /* `mars_stn_due_time`: nothing is waiting */
#define MARS_STN_ERR_NULL_CONFIG (-5) /* `mars_stn_create_longlink` got none  */

/* --- the eighteen questions, and their answers --------------------------- */

/**
 * Which of the eighteen questions STN asked.
 *
 * The integers are this ABI's own: they name a question, the way the C++'s
 * eighteen virtuals do by name.
 */
typedef enum {
    MarsStnQuestionNothing = 0,
    MarsStnQuestionMakesureAuthed = 1,     /* is the app logged in for host/user? */
    MarsStnQuestionTrafficData = 2,        /* how much went out and came in      */
    MarsStnQuestionOnNewDns = 3,           /* the ips the app knows for a host   */
    MarsStnQuestionOnPush = 4,             /* something no task asked for        */
    MarsStnQuestionReq2Buf = 5,            /* what a task is to send             */
    MarsStnQuestionBuf2Resp = 6,           /* how an answer is read              */
    MarsStnQuestionOnTaskEnd = 7,          /* a task that is over                */
    MarsStnQuestionReportConnectStatus = 8,
    MarsStnQuestionLongLinkNetworkError = 9,
    MarsStnQuestionShortLinkNetworkError = 10,
    MarsStnQuestionLongLinkStatusChange = 11,
    MarsStnQuestionIdentifyCheckBuffer = 12, /* the check a new link is used with */
    MarsStnQuestionIdentifyResponse = 13,
    MarsStnQuestionRequestSync = 14,
    MarsStnQuestionNetCheckShortLinkHosts = 15,
    MarsStnQuestionReportTaskProfile = 16,
    MarsStnQuestionReportTaskLimited = 17,
    MarsStnQuestionReportDnsProfile = 18
} MarsStnQuestionKind;

/**
 * Which answer the caller wrote. One answer per question: an answer of another
 * kind than the question asked for is no answer, and STN takes its own instead.
 */
typedef enum {
    MarsStnAnswerNothing = 0,   /* nobody answered                            */
    MarsStnAnswerYes = 1,       /* `yes` is 0 or 1                            */
    MarsStnAnswerIps = 2,       /* `ips` / `ip_count`                         */
    MarsStnAnswerEncoded = 3,   /* `bytes` / `byte_count` is what to send     */
    MarsStnAnswerFailed = 4,    /* `error_code` is what the task ends with    */
    MarsStnAnswerDecoded = 5,   /* `error_code` and `handle`                  */
    MarsStnAnswerEnded = 6,     /* `error_code`                               */
    MarsStnAnswerIdentified = 7, /* `mode`, `bytes`, `hash`, `cmdid`          */
    MarsStnAnswerLimit = 8      /* `limit`; 0 is "go ahead"                   */
} MarsStnAnswerKind;

/** One header of a task. */
typedef struct {
    const char* name;
    const char* value;
} MarsStnHeader;

/** A list of NUL-terminated strings, owned by the caller. */
typedef struct {
    const char* const* items;
    unsigned int count;
} MarsStnStrings;

/**
 * What a long link is made from: `LonglinkConfig` of mars/stn/stn.h with C types
 * inside.
 *
 * `name` is what every other call that takes one asks with; an empty `host_list`
 * means "the hosts the app set", an empty `group` the long-link group, and a
 * `link_type` of 0 `Task::CHANNEL_LONG` — the two defaults a zeroed struct is
 * filled with. The three flags are 0 or 1, and a C `int` has no "unset" for
 * them: a zeroed struct is a link with no TLS, which is the one place it is not
 * the default of `LonglinkConfig::new`, and that one asks for TLS.
 */
typedef struct {
    const char* name;
    MarsStnStrings host_list;
    int is_keep_alive;
    const char* group;
    int is_main;
    int link_type;
    int need_tls;
} MarsStnLonglinkConfig;

/**
 * One unit of work: `Task` of mars/stn/stn.h with C types inside.
 *
 * Every string and list is owned by the caller and read for the duration of the
 * call it was handed to. The `send_only` and friends flags are 0 or 1, and
 * `redirect_type` is 0 none, 1 bare to https, 2 http to https, 3 new host.
 */
typedef struct {
    unsigned int taskid;        /* `mars_stn_gen_task_id` hands one out      */
    unsigned int cmdid;         /* for a task that goes out on a long link    */
    unsigned long long channel_id;
    int channel_select;         /* one of the `Task::CHANNEL_*`; 0 is refused */
    int transport_protocol;     /* one of the `Task::TRANSPORT_PROTOCOL*`     */
    const char* cgi;            /* for a task that goes out on a short link   */
    int send_only;
    int need_authed;
    int limit_flow;
    int limit_frequency;
    int network_status_sensitive;
    int channel_strategy;
    int priority;
    int retry_count;            /* negative is the default                    */
    int server_process_cost;
    int total_timeout;
    int long_polling;
    int long_polling_timeout;
    const char* report_arg;
    const char* channel_name;
    const char* group_name;
    const char* user_id;
    int protocol;
    const MarsStnHeader* headers;
    unsigned int header_count;
    MarsStnStrings shortlink_host_list;
    MarsStnStrings shortlink_fallback_hostlist;
    MarsStnStrings longlink_host_list;
    MarsStnStrings minorlong_host_list;
    MarsStnStrings quic_host_list;
    int max_minorlinks;
    const char* function;
    const char* cgi_prefix;
    int redirect_type;
    unsigned short client_sequence_id;
} MarsStnTask;

/**
 * The connect a task ran on, as the app's report wants it: every reading is a
 * `gettickcount()`.
 */
typedef struct {
    unsigned long long start_time;
    unsigned long long start_connect_time;
    unsigned long long connect_successful_time;
    unsigned long long start_send_packet_time;
    unsigned long long send_packet_finished_time;
    unsigned long long start_read_packet_time;
    unsigned long long read_packet_finished_time;
    unsigned long long start_encode_packet_time;
    unsigned long long encode_packet_finished_time;
    unsigned long long start_decode_packet_time;
    unsigned long long decode_packet_finished_time;
    int channel_type;
    int transport_protocol;
    unsigned int rtt;
    const char* nettype;
} MarsStnCgiProfile;

/**
 * How a dns question went. `err_type` is one of `ErrCmdType`'s: 0 ok, 1 false,
 * 2 dial, 3 dns, 4 socket, 5 http, 6 netmsgxp, 7 endecode, 8 server, 9 local,
 * 10 canceld. `dnstype` is 1 the app's dns, 2 the platform's.
 */
typedef struct {
    unsigned long long start_time;
    unsigned long long end_time; /* 0 while it has not come back             */
    const char* host;
    int err_type;
    int err_code;
    int dnstype;
} MarsStnDnsProfile;

/**
 * One of the eighteen questions, in the arguments the C++ hands the app.
 *
 * Every field is read for the `kind` in it and left alone for the others, so a
 * caller switches on the kind and reads what it names: `host` is the host of
 * `MarsStnQuestionMakesureAuthed`, `MarsStnQuestionOnNewDns` and
 * `MarsStnQuestionShortLinkNetworkError`; `ip` / `port` the pair of the two
 * network errors; `channel_id` the link of `MarsStnQuestionOnPush`,
 * `MarsStnQuestionIdentifyCheckBuffer` and `MarsStnQuestionIdentifyResponse`;
 * `body` what was pushed, what came back and what the server answered. The
 * strings and buffers are owned by STN and are only good while the ask runs.
 */
typedef struct {
    MarsStnQuestionKind kind;
    const char* host;
    const char* user_id;
    const char* channel_id;
    const char* ip;
    unsigned short port;
    unsigned int cmdid;
    unsigned int taskid;
    long long send;
    long long recv;
    int channel_select;
    unsigned short sequence;
    const unsigned char* body;
    unsigned int body_count;
    const unsigned char* hash;
    unsigned int hash_count;
    int err_type;
    int err_code;
    const MarsStnCgiProfile* profile;
    const char* profile_json;
    const MarsStnDnsProfile* dns;
    int net_status_all;       /* one of `NetStatus`'s: -1 unknown … 5 down   */
    int net_status_longlink;
    int link_status;          /* one of `LongLinkStatus`'s: 0 idle … 4 failed */
    int longlink_host;
    int check_type;
    const MarsStnTask* task;
    unsigned int limit;       /* what the gate weighed the task against      */
} MarsStnQuestion;

/**
 * What the caller answered. Every field is read for the `kind` in it and left
 * alone for the others, so a caller fills in the one it answered and leaves the
 * rest zero.
 *
 * The strings and buffers it points at are the caller's, and they are read
 * *after* the ask has returned: the port copies them there, because there is
 * nowhere to copy them from while the callback is still running. So they have
 * to stay alive until the caller is asked again, and not merely until the ask
 * returns — an answer made of stack buffers is one whose buffers are gone by
 * the time it is read. Give them static or heap storage, the way
 * `tests/stn_smoke.rs` and the Swift of `MarsStn` do.
 */
typedef struct {
    MarsStnAnswerKind kind;
    int yes;
    const char* const* ips;
    unsigned int ip_count;
    const unsigned char* bytes;
    unsigned int byte_count;
    int error_code;
    int handle;               /* one of the `kTaskFailHandle*` ints           */
    int mode;                 /* 0 now, 1 next connect, anything else never   */
    const unsigned char* hash;
    unsigned int hash_count;
    unsigned int cmdid;
    unsigned int limit;
} MarsStnAnswer;

/**
 * The app STN asks: one question in, one answer out, and `ctx` — what the
 * caller handed to `mars_stn_set_app` — handed back with every one.
 *
 * What the answer points at has to outlive the call: see `MarsStnAnswer`.
 *
 * `NULL` is an app that answers nothing, which gets STN's own answers.
 */
typedef void (*MarsStnAsk)(void* ctx, const MarsStnQuestion* question, MarsStnAnswer* answer);

/* --- the task pipeline --------------------------------------------------- */

/**
 * `SetCallback` — the app STN talks to: one function pointer the eighteen
 * questions are funnelled through, and `ctx`, which is handed back with every
 * one of them. A `NULL` `ask` is an app that answers nothing.
 *
 * The app is asked while the process-wide pipeline is held, so `ask` must not
 * call another `mars_stn_*`.
 */
void mars_stn_set_app(void* ctx, MarsStnAsk ask);

/**
 * `Reset` — a net core made again from nothing: the tasks, the signalling
 * session and the addresses the setters handed to the net source are gone with
 * the one before it. The app is not: it is the caller's.
 */
void mars_stn_reset(void);

/** `ResetAndInitEncoderVersion` — reset plus the encoder of the new net core. */
void mars_stn_reset_and_init_encoder_version(int version, const char* name);

/**
 * `SetLonglinkSvrAddr` — the host and ports the long link goes out on, and the
 * ip `debug_ip` makes it reach without asking dns ("" or NULL for none).
 */
void mars_stn_set_longlink_svr_addr(const char* host,
                                    const unsigned short* ports,
                                    unsigned int port_count,
                                    const char* debug_ip);

/** `SetShortlinkSvrAddr`. */
void mars_stn_set_shortlink_svr_addr(unsigned short port, const char* debug_ip);

/** `SetDebugIP` — a host reached without asking dns; an empty ip drops it. */
void mars_stn_set_debug_ip(const char* host, const char* ip);

/** `SetBackupIPs` — the ips a host falls back to; an empty list drops the host. */
void mars_stn_set_backup_ips(const char* host, const char* const* ips, unsigned int ip_count);

/**
 * `StartTask` — one unit of work.
 *
 * @return MARS_STN_OK, or MARS_STN_ERR_NULL_TASK, or MARS_STN_ERR_REFUSED when
 *         the task is not one the queues would take — no channel to go out on,
 *         a timeout the C++ refuses — or MARS_STN_ERR_PANIC.
 */
int mars_stn_start_task(const MarsStnTask* task);

/** `StopTask` — 1 when it was one of ours, 0 when it was not. */
int mars_stn_stop_task(unsigned int taskid);

/** `HasTask` — whether the task is in one of the queues: 1 or 0. */
int mars_stn_has_task(unsigned int taskid);

/** `RedoTask` — every task that is out is run again. */
void mars_stn_redo_tasks(void);

/** `TouchTasks` — the queues are sorted again. */
void mars_stn_touch_tasks(void);

/** `ClearTask` — every task that is out is thrown away. */
void mars_stn_clear_tasks(void);

/**
 * What the C++'s message queue thread would have done: the follow-ups, one at a
 * time in the order they were posted, and then one pass of everything the two
 * queues and the zombies only do when they are asked — a task's first-package
 * timeout is one of those things, and a zombie that is started again is
 * another.
 *
 * The C++ runs this on threads of its own; this port has none, so it is the
 * host's loop that calls it, and `mars_stn_due_time` is how long it may wait.
 * A task that is
 * started and never drained sits in its queue until the process ends, which is
 * why the two are one pair: the header that declares `mars_stn_start_task` and
 * none of these leaves a caller a pipeline it can fill and never empty.
 */
void mars_stn_run_pending(void);

/**
 * `NetCore::GetNextHeartbeatTime` — how long the host's loop may wait before it
 * calls `mars_stn_run_pending` again: the soonest of the two queues, the zombie
 * check and the timing sync's alarm, as milliseconds left. Not as the
 * `gettickcount()` those are measured in, which is a reading of a clock this
 * library keeps to itself and no caller can subtract from.
 *
 * @return that many milliseconds — 0 is a pass that is already due, which a
 *         follow-up waiting in the queue is — or MARS_STN_ERR_NO_DUE when there
 *         is nothing to wait for — no task is out, no zombie is being checked,
 *         no alarm is armed — or MARS_STN_ERR_PANIC.
 */
long long mars_stn_due_time(void);

/** `MakesureLongLinkConnected` — 1 when there was a default link to connect. */
int mars_stn_makesure_longlink_connected(void);

/**
 * `MakesureLonglinkConnected_ext` — the same for the link `name` was made with.
 * A name no link was made with is nothing at all.
 */
void mars_stn_makesure_longlink_connected_ext(const char* name);

/**
 * `LongLinkIsConnected` — whether the default long link is up: 1 or 0.
 *
 * What "up" is is `LongLink::kConnected` and nothing else, so a link that is
 * still connecting answers 0.
 */
int mars_stn_longlink_is_connected(void);

/** `LongLinkIsConnected_ext` — the same for `name`, 0 for one no link has. */
int mars_stn_longlink_is_connected_ext(const char* name);

/* --- the long links the app names ---------------------------------------- */

/**
 * `CreateLonglink_ext` — a long link the caller named, made the way the default
 * one was. A name a link already has is that link and not a second one.
 *
 * @return MARS_STN_OK, MARS_STN_ERR_NULL_CONFIG, or MARS_STN_ERR_REFUSED when
 *         there is no net core, the long link is off, or the factory would make
 *         no link — a shape the C++ has no answer for, its
 *         `CreateLonglink_ext` being void — or MARS_STN_ERR_PANIC.
 */
int mars_stn_create_longlink(const MarsStnLonglinkConfig* config);

/** `DestroyLonglink_ext` — 1 when a link of that name was there, 0 when not. */
int mars_stn_destroy_longlink(const char* name);

/**
 * `MarkMainLonglink_ext` — the link of that name is the one whose errors and
 * status the app is told about, and the one "the" long link is.
 *
 * @return 1 when it is now the main one, 0 when no link has that name or it
 *         already was.
 */
int mars_stn_mark_main_longlink(const char* name);

/**
 * `DisableLongLink` — no task goes out on a long link again.
 *
 * The C++'s is a one-way door: only `mars_stn_reset`, which makes a net core
 * from nothing, opens it again.
 */
void mars_stn_disable_longlink(void);

/**
 * `getNoopTaskID` — the taskid of the noop, the one task no app started.
 *
 * `OnPush` and `Req2Buf` are asked for it too, so an app that cannot tell it
 * apart from its own tasks would answer it.
 */
unsigned int mars_stn_noop_task_id(void);

/**
 * `SetSignallingStrategy` — for every keeper in the process. A period or a keep
 * time of 0 leaves the `SignallingKeeper` defaults alone.
 */
void mars_stn_set_signalling_strategy(long long period, long long keep_time);

/** `KeepSignalling`. */
void mars_stn_keep_signalling(void);

/** `StopSignalling`. */
void mars_stn_stop_signalling(void);

/** `SetClientVersion` — the version every long-link package goes out with. */
void mars_stn_set_client_version(unsigned int version);

/** `GenTaskID` — one counter for the whole process. */
unsigned int mars_stn_gen_task_id(void);

/** `GenSequenceId` — an `unsigned short`, like the C++'s. */
unsigned short mars_stn_gen_sequence_id(void);

/** `TrigNooping` — a noop on the default long link, and a heartbeat of 0. */
void mars_stn_trig_nooping(void);

/**
 * `ActiveLogic::OnForeground(_isforeground)` — the app came to the front, or
 * left it, which is what a task asks before it wakes a long link that is down
 * and what the anti-avalanche check and the timing sync are told about. This is
 * `BaseEvent.onForeground` on Android and one call of
 * `mars::baseevent::OnForeground` everywhere else: there is no `BaseEvent` in an
 * ABI of plain functions, so a host of this header calls it itself.
 *
 * A host that never calls it gets the C++'s `ActiveLogic` as it is made: not in
 * front, so nothing is woken for a task. A change makes the app active again
 * whichever way it went, and ten minutes in the background end that — the C++
 * counts them on `alarm_`, and `mars_stn_run_pending` counts them here.
 *
 * @param is_foreground 0 for the background, anything else for the front.
 */
void mars_stn_on_foreground(int is_foreground);

/**
 * `mars::baseevent::GetSignalOnNetworkChange()` — the network under the app
 * changed, which is `BaseEvent.onNetworkChange` on Android: every long link is
 * taken down and made again, and dns is asked afresh, because the ip the last
 * connect landed on is one the new network may not route to.
 */
void mars_stn_on_network_change(void);


#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MARS_STN_H_ */
