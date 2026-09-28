#!/usr/bin/env python3
"""Point the three podspecs at one release.

    .github/scripts/update_podspecs.py <tag>

What moves is the number a pod calls itself and, for the two pods that carry a
binary, the url `pod install` downloads it from: MarsRSXlog and MarsRSNet name
the release asset of this tag, so a pod is only resolvable once the release it
names is out. `MarsRS` names no url — its `source` is the git tag — but it does
pin the two halves to this version, which is the same number.

Nothing else is rewritten, and the three files are read one at a time: a
podspec is Ruby, and the port keeps no parser for one.
"""

import re
import sys

REPO = "https://github.com/orangeboyChen/mars-rs/releases/download"

tag = sys.argv[1]
version = tag[1:] if tag.startswith("v") else tag

for pod in ("MarsRS", "MarsRSXlog", "MarsRSNet"):
    path = "%s.podspec" % pod
    src = open(path).read()

    pattern = re.compile(r"(s\.version\s+= ')[^']+(')")
    src, count = pattern.subn(r"\g<1>%s\g<2>" % version, src)
    if count != 1:
        raise SystemExit("%s: expected one s.version, rewrote %d" % (path, count))

    # The umbrella carries no binary and so names no url; the two halves name
    # the archive `scripts/package_cocoapods.sh` wrote for this version.
    if pod != "MarsRS":
        half = pod[len("MarsRS") :].lower()
        url = "%s/%s/marsrs-cocoapods-%s-%s.zip" % (REPO, tag, half, version)
        pattern = re.compile(r"(:http => ')[^']+(')")
        src, count = pattern.subn(r"\g<1>%s\g<2>" % url, src)
        if count != 1:
            raise SystemExit("%s: expected one :http source, rewrote %d" % (path, count))

    open(path, "w").write(src)
