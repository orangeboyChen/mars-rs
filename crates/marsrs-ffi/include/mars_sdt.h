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
 * mars_sdt.h — C ABI of the Rust network diagnosis (`marsrs-ffi`, the `sdt`
 * feature).
 *
 * This header mirrors the surface of
 *
 *     mars/sdt/sdt.h                (SdtLogic: StartActiveCheck / CancelActiveCheck ...)
 *     mars/sdt/sdt_core.cc          (the `__RunOn` thread that runs the checks)
 *     mars/sdt/src/checkimpl/       (the four probes a check asks)
 *
 * and is the C equivalent of the JNI bridge of `mars/sdt`, its
 * `*_Java2C.cc` pair.
 *
 * The one thing a caller supplies is the network: DNS, TCP, HTTP and ping are
 * four probes this library refuses to open for itself, so they cross the
 * boundary as one function pointer — `MarsSdtProbe` — and `mars_sdt_run_checks`
 * drives the run the C++ drives from `__RunOn`. Answer nothing and the
 * check that asked is a failed one, and a failed check ends the run: what
 * stands behind it in the plan is not checked. A ping is the one exception
 * — a ping nobody sent is a check that did not run, so it is left out of
 * the report and the run goes on behind it. That is what a host with no
 * network gets.
 *
 * The header is hand-written and checked in next to the crate so that a C/C++
 * or Swift caller can include it without running cbindgen. It is kept in sync
 * with `src/sdt.rs` by `tests/sdt_header_sync.rs`.
 *
 * Threading: every symbol may be called from any thread, but the diagnosis is
 * one process-wide value — the counterpart of the C++'s singleton — and
 * `mars_sdt_run_checks` holds it for as long as the checks take. Everything a
 * run calls back into is therefore entered with that lock held, and must not
 * call another `mars_sdt_*` from inside: the probe, which is asked once per
 * check, and — on a seam that hands the report to an app — the app's own
 * handler for it. Either one asking `mars_sdt_is_checking()`,
 * `mars_sdt_plan()` or `mars_sdt_start_active_check()` waits for the lock the
 * run is holding, and the run is waiting for the call to come back. What a
 * probe needs is in the query it is given.
 *
 * Panics: no Rust panic ever crosses this boundary. Every entry point is
 * wrapped in `catch_unwind`; a panic is reported as `MARS_SDT_ERR_PANIC` (or
 * silently swallowed for the `void` symbols).
 */

#ifndef MARS_SDT_H_
#define MARS_SDT_H_

/* The symbols this header declares are the ones `#[no_mangle]` exports: plain
 * C names, and a C++ translation unit that includes this and calls
 * `mars_sdt_reset()` asks its linker for a mangled one that does not exist.
 * The guard says what the library already says for itself. */
#ifdef __cplusplus
extern "C" {
#endif

/* --- return codes ------------------------------------------------------- */

#define MARS_SDT_OK 0
#define MARS_SDT_ERR_PANIC (-1)      /* a Rust panic was caught at the boundary */
#define MARS_SDT_ERR_NULL_OUT (-2)   /* `out` was NULL                        */
#define MARS_SDT_ERR_NO_SPACE (-3)   /* output buffer too small (0 or < need) */
#define MARS_SDT_ERR_NO_PROBE (-4)   /* `mars_sdt_run_checks` got no probe    */
#define MARS_SDT_ERR_BUSY (-5)       /* a check is already in flight          */
#define MARS_SDT_ERR_NO_CHECK (-6)   /* no check recorded anything           */
#define MARS_SDT_ERR_BAD_ARG (-7)    /* the arguments cannot start a check    */

/* --- the bits a mode is made of ------------------------------------------ */

/* `NET_CHECK_BASIC` / `NET_CHECK_LONG` / `NET_CHECK_SHORT` of
 * mars/sdt/constants.h, which the `mode` of `mars_sdt_start_active_check` is
 * made of: they OR together, and a mode with none of them in it is no checks
 * at all — an empty plan, and MARS_SDT_ERR_BAD_ARG. It is not "run
 * everything", which is `NET_CHECK_BASIC | NET_CHECK_LONG | NET_CHECK_SHORT`.
 */
#define NET_CHECK_BASIC 1 /* ping and dns                                    */
#define NET_CHECK_LONG 2  /* tcp: a noop out to the long link's hosts        */
#define NET_CHECK_SHORT 4 /* http: the net-check CGI, the short link's hosts */

/* --- what a probe is asked, and what it answers -------------------------- */

/**
 * Which probe is being asked: the four of mars/sdt/src/checkimpl/, plus
 * `MarsSdtNothing` for a probe nobody answered: a failure for the check
 * that asked, and a skip for a ping.
 */
typedef enum {
    MarsSdtNothing = 0, /* nobody answered: a failure, but a ping is skipped */
    MarsSdtDns = 1,     /* socket_gethostbyname                              */
    MarsSdtTcp = 2,     /* TcpQuery: a long-link noop out, and what comes back */
    MarsSdtHttp = 3,    /* SendHttpQuery: the net-check CGI                  */
    MarsSdtPing = 4     /* PingQuery::RunPingQuery                           */
} MarsSdtKind;

/**
 * One check of a request: `NetCheckType` of mars/sdt/sdt.h, which is also the
 * `detectType` of every entry of the report — so a plan and a report name the
 * same checks with the same integers.
 *
 * These are *not* the integers of `MarsSdtKind`: that one is what a probe is
 * asked, this one is what a request is made of.
 */
typedef enum {
    MarsSdtCheckPing = 0,
    MarsSdtCheckDns = 1,
    MarsSdtCheckNewDns = 2,   /* the resolve against another server          */
    MarsSdtCheckTcp = 3,
    MarsSdtCheckHttp = 4,
    MarsSdtCheckTraceroute = 5, /* planned; no probe is asked for it yet     */
    MarsSdtCheckReqBuf = 6      /* planned; no probe is asked for it yet     */
} MarsSdtCheck;

/** What one probe is asked. */
typedef struct {
    MarsSdtKind kind;
    const char* host;   /* dns: the domain; tcp: the ip; http: the url; ping: the host */
    unsigned short port; /* tcp only                                         */
    unsigned int timeout; /* ms for the first three, s for a ping           */
} MarsSdtQuery;

/**
 * What one probe answers. Every field is read for the `kind` in it and ignored
 * for the others, so a caller fills in the one it answered and leaves the rest
 * zero. `rtt` is the C++'s `cost_time`.
 */
typedef struct {
    MarsSdtKind kind;
    int error_code;      /* the probe's return value; >= 0 is one that worked  */
    unsigned long long rtt; /* cost_time                                      */
    const char* const* ips; /* the addresses a resolve found                  */
    unsigned int ip_count;
    int sent;            /* tcp_send                                          */
    int received;        /* tcp_receive                                       */
    unsigned char is_noop_resp; /* 0 or 1                                     */
    int status_code;     /* the HTTP status of the net-check CGI's answer      */
    float loss_rate;     /* PingStatus::loss_rate; 1.0 is every ping lost      */
    float avgrtt;        /* PingStatus::avgrtt                                 */
} MarsSdtAnswer;

/** One host and port of a link: `CheckIPPort` of mars/sdt/sdt.h. */
typedef struct {
    const char* ip;
    unsigned short port;
} MarsSdtIpPort;

/** The hosts of one link, keyed by the name they are known under. */
typedef struct {
    const char* name;
    const MarsSdtIpPort* ports;
    unsigned int port_count;
} MarsSdtHosts;

/**
 * The four probes, asked the way the checkers ask them: one question in, one
 * answer out, and `ctx` — what the caller handed to `mars_sdt_run_checks` —
 * handed back with every one.
 */
typedef void (*MarsSdtProbe)(void* ctx, const MarsSdtQuery* query, MarsSdtAnswer* answer);

/* --- the diagnosis ------------------------------------------------------- */

/**
 * Throws the diagnosis away: the request, the check that may be in flight and
 * everything that was reported go away together, which is `SdtLogic::Reset`.
 */
void mars_sdt_reset(void);

/** `SetHttpNetcheckCGI` — the URL the HTTP check goes to; NULL clears it. */
void mars_sdt_set_http_netcheck_cgi(const char* cgi);

/**
 * The URL `mars_sdt_set_http_netcheck_cgi` set.
 *
 * @return the number of bytes written excluding the terminating NUL, or a
 *         negative MARS_SDT_ERR_* code (`MARS_SDT_ERR_NULL_OUT`,
 *         `MARS_SDT_ERR_NO_SPACE`, `MARS_SDT_ERR_PANIC`).
 */
int mars_sdt_http_netcheck_cgi(char* out, unsigned int len);

/**
 * `StartActiveCheck` — a diagnosis of the two links' hosts, in `mode` and with
 * `timeout` milliseconds to spend on it.
 *
 * @return MARS_SDT_OK, or MARS_SDT_ERR_BUSY when a check is already in flight —
 *         the one answer a caller retries — or MARS_SDT_ERR_BAD_ARG when these
 *         arguments cannot start a check at all: a `longlink` / `shortlink`
 *         that promises `count` hosts behind a NULL pointer, or a `mode` with
 *         none of the three `NET_CHECK_*` bits in it, which is a request with an
 *         empty plan. The two are separate codes because retrying the first
 *         ends when the request in flight does and retrying the second never
 *         does, or MARS_SDT_ERR_PANIC.
 */
int mars_sdt_start_active_check(const MarsSdtHosts* longlink,
                                unsigned int longlink_count,
                                const MarsSdtHosts* shortlink,
                                unsigned int shortlink_count,
                                int mode,
                                unsigned int timeout);

/** `CancelActiveCheck` — the check in flight is asked to stop. */
void mars_sdt_cancel_active_check(void);

/** Whether a check is in flight: 1 or 0. */
int mars_sdt_is_checking(void);

/**
 * The checks the request is going to make, in order: as many `MarsSdtCheck`
 * as there are checks, written into `out`.
 *
 * @return how many checks there are, whether or not they all fit: `out` may be
 *         NULL and `cap` may be 0, which is how a caller asks for the size.
 */
unsigned int mars_sdt_plan(MarsSdtCheck* out, unsigned int cap);

/**
 * Runs the planned checks — one probe per check, in order — and reports what
 * they recorded. This is the `__RunOn` thread of the C++, driven by the caller.
 *
 * `probe` may be NULL, which is reported as MARS_SDT_ERR_NO_PROBE; the request
 * in flight is cancelled, so the next MARS_SDT_START is taken rather than
 * answered MARS_SDT_ERR_BUSY for the life of the process.
 *
 * MARS_SDT_ERR_NO_CHECK is what a run that recorded nothing answers — nothing
 * was in flight, the request was cancelled before its first check, or the
 * checks it planned had nothing to check — and not "the run failed": the
 * request is over and the report for it is an empty one.
 * `network_type` is the `comm::getNetInfo()` every check writes into its
 * profiles, which is the platform's to answer.
 *
 * @return MARS_SDT_OK, or MARS_SDT_ERR_NO_PROBE, MARS_SDT_ERR_NO_CHECK or
 *         MARS_SDT_ERR_PANIC.
 */
int mars_sdt_run_checks(void* ctx, MarsSdtProbe probe, int network_type);

/**
 * Takes the JSON report of everything the checks have reported since the last
 * call — `{"details":[ … ]}`, one object per host a check probed, of every
 * result recorded since the last take.
 *
 * Not the document `SdtLogic.reportSignalDetectResults(String)` gets in the
 * C++, which is the one of the run that just finished: the callback records a
 * run's results, and a take hands over however many runs are waiting.
 *
 * @return the number of bytes written excluding the terminating NUL, or a
 *         negative MARS_SDT_ERR_* code.
 */
int mars_sdt_take_report(char* out, unsigned int len);


#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MARS_SDT_H_ */
