# The xlog CLI

`xlog` is one file format with a command line in front of it: it writes a
`.xlog`, it reads one back, and it makes the pair of keys that decides who can
read the one it wrote.

An app never needs it — the appender inside the app writes the same bytes — but a
shell does. A log pulled off a device, a log on its way to a server, a test that
needs a `.xlog` to read, and the key pair nobody has made yet are all one command
away.

::: code-group

```bash [From the tag]
# The crate is not on crates.io yet — publication is pending — so the tag is
# what `cargo install` is pointed at. It puts the `xlog` bin on the $PATH.
cargo install --git https://github.com/orangeboyChen/mars-rs --tag v0.1.0-alpha.3 marsrs-xlog
xlog --version
```

```bash [From a release]
# marsrs-xlog-cli-<version>-<host>.tar.gz, or .zip on Windows
tar -xzf marsrs-xlog-cli-0.1.0-alpha.3-aarch64-apple-darwin.tar.gz
./marsrs-xlog-cli-0.1.0-alpha.3-aarch64-apple-darwin/xlog --version
```

:::

The archive is built for the same three hosts as the C ABI's — Linux, macOS and
Windows — and carries one command and nothing else; it is for the machine that
has no Rust toolchain. The tag and the archive give the same `xlog`.

| command | what it does |
|---|---|
| `xlog keygen` | makes a key pair: a public key for a config, and the private key that reads what it wrote |
| `xlog encode` | writes a `.xlog` out of one record per line of its input |
| `xlog decode` | reads one back and prints the log text |

`xlog help` prints the whole command line — every option, and every default — and
`xlog --version` the version. A subcommand and an option each have a short
spelling: `xlog k` is `xlog keygen`, `xlog e -p <hex>` is
`xlog encode --pubkey=<hex>`, and `-oFILE`, `-o=FILE` and `-o FILE` are three
spellings of one option.

## Making a key pair

An appender encrypts when its config carries a public key: every async record's
body is encrypted with a key the writer and the reader agree on through ECDH, and
only the private key of that pair undoes it. Nothing in the port holds a pair of
its own, so the pair is yours to make — once, before the app that uses it ships:

```bash
$ xlog keygen
pubkey=e5a1c9…88f0        # 128 hex characters
privkey=3c07b2…41de       # 64 hex characters

$ xlog keygen --out=xlog.key     # the same two lines, in a file
```

The public key is what a config's `pubKey` takes — the same 128 hex characters on
every platform. The private key is what `xlog decode --privkey` takes, and the
only thing that reads those logs back: it is not in the app, and it is not in the
file either, so a pair nobody wrote down is a log nobody can read.

`--out` creates that file for you alone — `0600` on Unix — and will not write
over one that is already there. A private key is the only thing that reads every
log it was the pair of, so giving it away with the file, or replacing it, is
losing them.

Two runs make two pairs — a pair is drawn from the system's generator and kept
nowhere — so make one and keep it, the way a deploy key is kept.

A sync record is neither compressed nor encrypted, which is what the C++ writes:
a `--sync` file is readable with no key at all, whatever the config says.

## Reading a file back

```bash
xlog decode --privkey=<hex> marsrs_20260927.xlog --out=marsrs.plain
```

A file written with no public key is read with no option at all. An encrypted
record met without the private key is an error that names the record, and not a
record that is silently skipped: a file either gives its records back or says why
it cannot.

`INPUT` is a path or `-`, and `--out` is a path, `-`, or left out — so
`xlog decode a.xlog | less` works, and so does `xlog decode < a.xlog > a.plain`.

This is upstream's `decode_mars_log_file.py` over the same bytes, so a `.xlog`
written by the C++ implementation is read here and one written here is read
there.

## Writing one

```bash
xlog encode --pubkey=<hex> records.txt --out=marsrs_20260927.xlog
```

One record per line of the input, the line's own newline included — which is what
makes `encode` of a file and `decode` of the result give the file back.

| option | what it does | default |
|---|---|---|
| `-p, --pubkey=HEX` | the public key of the pair; without it the file is written in the clear | none — no encryption |
| `-m, --mode=zlib\|zstd` | which compressor an async body is framed with | `zlib` |
| `-s, --sync=0\|1` | one record per block instead of one block per file | `0` |
| `-c, --compress=0\|1` | compress the payload. An async body is framed either way, so `--compress=0` goes with `--sync=1` | `1` |
| `-l, --level=N` | the zstd level | `6` |
| `-r, --region=N` | the size of the buffer a record is written through | `153600` |

What it writes is the file the C++ implementation writes — same magic, same
framing, same compression — which is the reason to write one: a `.xlog` to test a
reader against, or a fixture for the build that uploads them.

## When a command line is refused

`xlog` exits non-zero and says why on standard error. An option that belongs to
another subcommand, an input that holds no record, a public key that is not one —
each is refused before a byte is written, because a file written from a command
line that was not understood is worse than no file.
