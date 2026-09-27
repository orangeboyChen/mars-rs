"""The two comparisons `scripts/compat/cross.sh` makes.

    python3 scripts/compat/check.py exact <expected> <actual>
    python3 scripts/compat/check.py lines <records> <decoded>

`exact` is the record-level one: the bytes the decoder produced have to be the
bytes that went in, which is what the golden fixtures assert too.

`lines` is the appender-level one. A file the appender wrote carries more than
the records — its own startup banner, and a timestamped header in front of every
record — so what is checked is that every record is *in* it. Records the Rust
`&str` API cannot carry (the byte-soup record of `inputs.bin` is not UTF-8) are
skipped here and covered by `exact` at the record level instead.
"""

import sys


def exact(expected_path, actual_path):
    with open(expected_path, "rb") as handle:
        expected = handle.read()
    with open(actual_path, "rb") as handle:
        actual = handle.read()
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
    for record in records:
        try:
            text = record.decode("utf-8")
        except UnicodeDecodeError:
            continue  # not writable through the Rust appender's `&str`
        if text.encode("utf-8") not in decoded:
            missing.append(record)

    if missing:
        for record in missing:
            print(f"{decoded_path}: record missing: {record[:60]!r}")
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
