#!/usr/bin/env python3
"""Point the two binary targets of Package.swift at one release.

    scripts/update_package_swift.py <tag> <checksum> <asset> \
                                    <checksum-net> <asset-net>

One url and one checksum per artifact: two binary targets, and a checksum is
bound to the zip it was computed from, so the xlog framework's is not the net
one's. The asset each target names is matched by the module, which is what does
not change when a release renames what it publishes.
"""

import re
import sys

REPO = "https://github.com/orangeboyChen/mars-rs/releases/download"

tag, checksum, asset, checksum_net, asset_net = sys.argv[1:6]
targets = (
    ("MarsRSFFI", asset, checksum),
    ("MarsRSNetFFI", asset_net, checksum_net),
)

src = open("Package.swift").read()
for module, name, digest in targets:
    url = "%s/%s/%s" % (REPO, tag, name)
    pattern = re.compile(
        r'(name: "%s",)(\s*url: ")[^"]*(",\s*checksum: ")[0-9a-f]+(")' % module
    )

    def replacement(matched, url=url, digest=digest):
        return (
            matched.group(1)
            + matched.group(2)
            + url
            + matched.group(3)
            + digest
            + matched.group(4)
        )

    src, count = pattern.subn(replacement, src)
    if count != 1:
        raise SystemExit(
            "Package.swift: expected one binary target named %s, found %d"
            % (module, count)
        )
open("Package.swift", "w").write(src)
