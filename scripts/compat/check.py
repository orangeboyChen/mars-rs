"""The two comparisons `scripts/compat/cross.sh` makes.

    python3 scripts/compat/check.py exact <expected> <actual>
    python3 scripts/compat/check.py lines <records> <decoded>

`exact` is the record-level one: the bytes the decoder produced have to be the
bytes that went in, which is what the golden fixtures assert too.

`lines` is the appender-level one. A file the appender wrote carries more than
the records — its own startup banner, and a timestamped header in front of every
record — so what is checked is that every record is *in* it, framed the way the
appender framed it: `][` where the last header field ends, and the newline that
ends the record. The bare text is not enough to go on, because one fixture
record is inside another (`1234567` in `12345678`) and another is inside the
metadata around it (`x` in `xlog`), so a substring search passes on a file that
dropped them. Records the Rust `&str` API cannot carry (the byte-soup record of
`inputs.bin` is not UTF-8) are skipped here and covered by `exact` at the record
level instead.
"""

import collections
import sys


def exact(expected_path, actual_path):
    with open(expected_path, "rb") as handle:
        expected = handle.read()
    try:
        with open(actual_path, "rb") as handle:
            actual = handle.read()
    except OSError as err:
        # Upstream's decoder writes no output at all — and still exits 0 — when
        # it cannot read the file it was handed, so this is a shape the control
        # branch of `cross.sh` meets for real.
        print(f"{actual_path}: not readable: {err.strerror}")
        return 1
    if expected == actual:
        return 0
    print(f"{actual_path}: {len(expected)} bytes in, {len(actual)} bytes out")
    for index in range(min(len(expected), len(actual))):
        if expected[index] != actual[index]:
            print(f"  first difference at {index}: "
                  f"{expected[index]:#04x} -> {actual[index]:#04x}")
            break
    return 1


def lines(records_path, decoded_path):
    with open(records_path, "rb") as handle:
        records = [line for line in handle.read().split(b"\n") if line]
    with open(decoded_path, "rb") as handle:
        decoded = handle.read()

    missing = []
    checked = 0
    for record, expected in collections.Counter(records).items():
        try:
            text = record.decode("utf-8")
        except UnicodeDecodeError:
            continue  # not writable through the Rust appender's `&str`
        checked += 1
        # One framed record: what the appender stamps in front of the body, the
        # body, and the newline that ends it.
        framed = b"][" + text.encode("utf-8") + b"\n"
        found = decoded.count(framed)
        if found < expected:
            missing.append((record, expected, found))

    # A records file of nothing but records that were skipped is a comparison
    # of nothing: `missing` stays empty over a file no record was looked for
    # in, and `0` means "every record is there". One of the two has to have
    # been asked about for the answer to mean anything.
    if checked == 0:
        print(f"{decoded_path}: no record of {records_path} was checked — "
              "every one of them is a byte string the Rust appender cannot "
              "write, so nothing was compared")
        return 1

    if missing:
        for record, expected, found in missing:
            print(f"{decoded_path}: record written {found} of {expected} "
                  f"times: {record[:60]!r}")
        return 1
    return 0


def main(argv):
    if len(argv) != 4:
        print(__doc__.strip())
        return 2
    if argv[1] == "exact":
        return exact(argv[2], argv[3])
    if argv[1] == "lines":
        return lines(argv[2], argv[3])
    print(__doc__.strip())
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
