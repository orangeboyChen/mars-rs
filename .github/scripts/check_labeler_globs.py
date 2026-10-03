#!/usr/bin/env python3
"""Fail on a glob of .github/labeler.yml that no file of the tree matches.

    .github/scripts/check_labeler_globs.py

A glob that names a directory that is not there is a rule that never fires,
and a pull request that goes unlabelled is indistinguishable from one that
needed no label: the label is simply absent, and nothing in the run says
whether that was the answer or a mistake. Eighteen of the globs were in that
state, went on being in it for as long as the five platform trees had been
under `platforms/`, and were found by reading the file and not by a run.

So the question is asked of every glob, against every path `git ls-files`
knows. It is a subset of what `actions/labeler` matches with — `@actions/glob`
over a path — and the one this file writes: `**` spanning directories, `*`
and `?` within one segment, and `dir/**` meaning the directory and everything
under it. A glob no path matches is reported as a GitHub annotation and the
exit is 1.
"""

import fnmatch
import subprocess
import sys

LABELER = ".github/labeler.yml"


def matches(path: str, pattern: str) -> bool:
    """Whether a repository-relative path is one the glob names."""
    pattern = pattern[2:] if pattern.startswith("./") else pattern
    pats = pattern.split("/")
    segs = path.split("/")

    def walk(i: int, j: int) -> bool:
        if i == len(pats):
            return j == len(segs)
        if pats[i] == "**":
            # Zero segments or all of them: `a/**/b` is `a/b`, and `dir/**`
            # is `dir` itself.
            return any(walk(i + 1, k) for k in range(j, len(segs) + 1))
        if j >= len(segs):
            return False
        if fnmatch.fnmatchcase(segs[j], pats[i]):
            return walk(i + 1, j + 1)
        return False

    return walk(0, 0)


def globs(config: str):
    """Every (label, glob) the configuration names, in the order written.

    Read out of the lines and not with a YAML parser, because a parser is a
    package this job's runner does not carry and would have to be asked to
    install for one file: what the configuration names is one shape written
    the same way every time — a label, then `changed-files`, then
    `any-glob-to-any-file`, then the globs — and the indentation says which
    is which. A key ends in a colon and is not a glob; anything else under a
    label is one.
    """
    label = None
    for raw in config.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if not line.startswith("-"):
            # `'area: xlog':` — a label, and the only key at this depth
            label = line.rstrip(":").strip("'\"")
        elif not line.endswith(":"):
            # `- crates/**`, or `- '*.podspec'` with its quotes
            yield label, line[1:].strip().strip("'\"")


def main() -> int:
    paths = subprocess.run(
        ["git", "ls-files"], capture_output=True, text=True, check=True
    ).stdout.split()
    dead = [
        (label, glob)
        for label, glob in globs(open(LABELER).read())
        if not any(matches(path, glob) for path in paths)
    ]
    for label, glob in dead:
        print("::error file=%s::'%s' of '%s' matches no file in the tree" % (LABELER, glob, label))
    if dead:
        print(
            "\n%d of the globs in %s match nothing. A glob that names a"
            " directory that is not there is a rule that never fires."
            % (len(dead), LABELER)
        )
        return 1
    print("every glob in %s matches a file of the tree" % LABELER)
    return 0


if __name__ == "__main__":
    sys.exit(main())
