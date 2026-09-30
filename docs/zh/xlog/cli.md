# xlog 命令行

`xlog` 是一个文件格式，前面还有一层命令行：写 `.xlog`、读 `.xlog`，以及造出那对密钥
—— 它写的文件谁能读回来，就是那对密钥说了算。

App 从来不需要它 —— app 里的 appender 写出的是同样的字节 —— 但 shell 需要。从设备
上拖下来的日志、要传到服务端的日志、需要一个 `.xlog` 才能测的测试，还有那对还没造
过的密钥，都只差这一条命令。

::: code-group

```bash [从 tag]
# 这个 crate 还没上 crates.io —— 发布还在进行中 —— 所以 `cargo install` 指向的是
# tag。它把 `xlog` 装到 $PATH 上。
cargo install --git https://github.com/orangeboyChen/mars-rs --tag v0.1.0-alpha.3 marsrs-xlog
xlog --version
```

```bash [从 release]
# marsrs-xlog-cli-<version>-<host>.tar.gz，Windows 上是 .zip
tar -xzf marsrs-xlog-cli-0.1.0-alpha.3-aarch64-apple-darwin.tar.gz
./marsrs-xlog-cli-0.1.0-alpha.3-aarch64-apple-darwin/xlog --version
```

:::

压缩包和 C ABI 的一样，为 Linux、macOS、Windows 三个 host 构建，里面只有这一条命令，
别的什么都没有 —— 它是给没有 Rust 工具链的机器准备的。从 tag 装和从压缩包取，是同
一个 `xlog`。

| 命令 | 做什么 |
|---|---|
| `xlog keygen` | 造一对密钥：给配置用的公钥，和读回它写的日志的私钥 |
| `xlog encode` | 把输入里一行一条记录写成 `.xlog` |
| `xlog decode` | 读一个回来，打印里面的日志正文 |

`xlog help` 会把整个命令行 —— 每个选项、每个默认值 —— 打出来，`xlog --version` 打版本。
子命令和选项都有短写法：`xlog k` 就是 `xlog keygen`，`xlog e -p <hex>` 就是
`xlog encode --pubkey=<hex>`，而 `-oFILE`、`-o=FILE`、`-o FILE` 是同一个选项的三种
写法。

## 造一对密钥

配置里带了公钥，appender 就加密：每条 async 记录的正文用写方和读方通过 ECDH 协商出来的
密钥加密，只有那对密钥里的私钥能解开。这个移植自己不持有任何一对密钥，所以这对密钥
是你自己的事 —— 造一次，在用它的那个 app 发版之前：

```bash
$ xlog keygen
pubkey=e5a1c9…88f0        # 128 个十六进制字符
privkey=3c07b2…41de       # 64 个十六进制字符

$ xlog keygen --out=xlog.key     # 同样的两行，写进文件
```

公钥就是配置里 `pubKey` 要填的东西 —— 每个平台上都是同样这 128 个十六进制字符。私钥是
`xlog decode --privkey` 要接的东西，也是唯一能把那些日志读回来的东西：它不在 app 里，
也不在文件里，所以那对密钥要是没人记下来，就是谁也读不了的日志。

`--out` 建出来的文件只属于你 —— Unix 上是 `0600` —— 而且已经存在的文件它不会覆盖。
那对密钥写过的所有日志，只有这把私钥读得出来。所以把它随文件一起送出去、或者把它换掉，
都等于丢掉那些日志。

跑两次就是两对 —— 一对密钥是从系统的随机数生成器里取出来的，哪儿都不存 —— 所以造一对、
存好它，就像存一把部署密钥那样。

sync 记录既不压缩也不加密，C++ 也是这么写的：不管配置里写了什么，`--sync` 写出来的
文件没有密钥也能读。

## 读回来

```bash
xlog decode --privkey=<hex> marsrs_20260927.xlog --out=marsrs.plain
```

没有公钥写出来的文件，不需要任何选项就能读。碰到加密过的记录却没有私钥，那是一个
点名到那条记录的错误，而不是悄悄跳过去：一个文件要么把记录还给你，要么说清楚为什么
还不了。用**另一对**密钥去读也是一样 —— 每条记录都读不出来，那是“读了但什么都没读
出来”的文件，命令会说清楚，而不是交给你一份全是标记物的日志。

`INPUT` 是路径或 `-`；`--out` 是路径、`-`，或者不写 —— 所以
`xlog decode a.xlog | less` 可以，
`xlog decode < a.xlog > a.plain` 也可以。空值和 `-` 是一个意思：`--in=` 是标准输入，
`--out=` 是标准输出。

`--` 结束选项，它后面不管以什么开头都是输入：`xlog encode -- -weird.xlog` 读的是这个
名字的文件，而不是去找一个叫 `-w` 的选项。

这跟上游的 `decode_mars_log_file.py` 跑在同一批字节上：C++ 实现写的 `.xlog` 这里能读，
这里写的那边也能读。

## 写一个

```bash
xlog encode --pubkey=<hex> records.txt --out=marsrs_20260927.xlog
```

输入里一行就是一条记录，连这行自己的换行符也算进去 —— 正因为如此，对一个文件
`encode`、再对结果 `decode`，拿回来的是同一个文件。

| 选项 | 做什么 | 默认值 |
|---|---|---|
| `-p, --pubkey=HEX` | 那对密钥的公钥；不给就写成明文 | 无 —— 不加密 |
| `-m, --mode=zlib\|zstd` | async 正文用哪个压缩器成帧 | `zlib` |
| `-s, --sync=0\|1` | 一条记录一个块，而不是一个文件一个块 | `0` |
| `-c, --compress=0\|1` | 压缩正文。async 正文无论如何都要成帧，所以 `--compress=0` 要配 `--sync=1` | `1` |
| `-l, --level=N` | zstd 的压缩级别 | `6` |
| `-r, --region=N` | 写一条记录所用的缓冲区大小 | `153600`，最大 `67108864` |

它写出来的就是 C++ 实现写出来的那个文件 —— 同样的 magic、同样的帧结构、同样的压缩 ——
这也是要写一个的理由：给读文件的程序做测试的 `.xlog`，或者给上传日志的构建做 fixture。

## 命令行被拒的时候

`xlog` 用非零退出码退出，并在标准错误上说为什么。属于别的子命令的选项、一条记录都没有的
输入、根本不是密钥的公钥 —— 每一个都在写出一个字节之前被拒掉，因为照一条没读懂的命令行
写出来的文件，比没有文件更糟。
