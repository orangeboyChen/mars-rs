# Cross-checked the other way

`cargo test -p marsrs-compat` proves one half of the promise: the 16 `.xlog`
files under `crates/marsrs-compat/fixtures` were written by the C++ encoders
and the Rust decoder reads every one back. The other half — a file this port
wrote, read by the C++ — needs the C++ in the tree, so it lives in a script
instead of a test:

```bash
sh scripts/compat/cross.sh
```

It builds upstream's own decoder (`mars/xlog/crypt/decode_log_file_c_impl/
decode_log_file.c`, patched with the fixtures' ECDH keys) and an encoder
(`scripts/compat/upstream_encode.cpp`) that mirrors `xlog-compat encode`
option for option, then runs the same 16 combinations — zlib/zstd × sync/async ×
crypt on/off × flush policy — in both directions: encode here and decode there,
encode there and decode here, byte for byte against `fixtures/expected.bin`.
It then does the same through the real appenders, so what is decoded is a
`<prefix>_YYYYMMDD.xlog` an app would ship and not just the block bytes.

| layer | rust → C++ | C++ → rust |
|---|---|---|
| record layer, 16 combinations | 12 ok, 2 `cpp-decoder`, 2 fail | 16 ok |
| appender layer, 8 combinations | 8 ok | 8 ok |

`cpp-decoder` is two of the four `zstd-async` cases, and it is not a difference
between the encoders: upstream's decoder reads both encoders' `zstd-async`
files the same wrong way — 4501 of 4510 bytes with one flush — while the Rust
decoder reads both whole. The script runs that control itself and says so
instead of failing, but only when the two wrong outputs are the *same* wrong
bytes; a Rust file that decodes to something else fails.

The two that fail are the `zstd-async` cases that flush once per record. There
the Rust encoder's zstd frame for the 4096-byte record is one byte shorter than
upstream's — one raw literal before the match where upstream writes two, two
spellings of the same 4096 bytes, which the Rust decoder reads back whole from
both files. It only shows up because upstream's decoder returns 17 and 18 bytes
instead of 4096 for that block, so the two files come out of it 431 and 432
bytes long. Everything else — all 8 appender combinations, including async and
zstd — is byte for byte.

Two things the run turns up that are worth knowing:

* upstream's `decode_log_file.c` **does not compile as it stands** —
  `zstdDecompress` reads a `lastPos` it never declares (line 212). The build
  patches one `size_t` into its copy under `target/`.
* the Rust appender's `appender_write` takes a `&str`, so a record that is not
  UTF-8 cannot be logged through it; the C++ `xlogger_Write` takes bytes. The
  appender layer of the test checks the records that are UTF-8 and the record
  layer carries the rest through both encoders unchanged.
