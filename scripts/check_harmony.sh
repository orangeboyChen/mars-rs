#!/usr/bin/env bash
#
# Checks platforms/harmonyos/marsrs-xlog without building it — the manifests a
# publish reads, the lists of NAPI names that have to agree with each other, and
# the ArkTS, which is the one source in the port nothing compiles.
#
#   scripts/check_harmony.sh
#
# DevEco's own gate is a 2 GB command-line toolchain that wants a JDK and a
# project around the module, and `hvigor assembleHar` does not run on a Linux
# runner without it. So the ArkTS is checked the two ways it can be checked
# here: as text, by this script, and as TypeScript, by
# scripts/check_harmony_arkts.sh. Between them:
#
#   * the four `json5` files parse — `oh-package.json5`, `build-profile.json5`,
#     `src/main/module.json5` — which is the manifest a publish reads and the
#     two hvigor reads;
#   * `oh-package.json5` carries the four fields ohpm requires and the two files
#     it looks for, and the name the release uploads under is the name in it;
#   * every export of `Index.ets` resolves to a file of the module that exports
#     it — the entry point a HAR ships is a list of names and nothing else;
#   * the three lists of NAPI methods are one list: the
#     `napi_property_descriptor` of `src/main/cpp/napi_init.cpp`, the
#     `export const` of `src/main/cpp/types/libmarsrs_xlog/index.d.ts`, and the
#     `xlogNapi.<name>(` of the ArkTS. Nothing keeps those three apart but the
#     names being the same, and a name in two of them is a call that answers
#     `undefined` in the app;
#   * `nm_modname` of the C is the `lib<name>.so` the ArkTS imports, is the
#     `types/lib<name>/` the declarations sit in, and is the `.so` the build
#     script writes;
#   * the ArkTS keeps to ArkTS: no `any`, no `unknown`, no `var`, no `delete`
#     of a property, no `eval`.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

python3 - "$root" <<'PY'
import json
import os
import re
import sys

root = sys.argv[1]
module = os.path.join(root, 'platforms/harmonyos/marsrs-xlog')

bad = []


def fail(message):
    bad.append(message)


def path(*parts):
    return os.path.join(module, *parts)


def read(*parts):
    with open(path(*parts)) as handle:
        return handle.read()


# The JSON5 this repository writes is JSON with comments and trailing commas in
# it, which is the whole of the difference: stripped of those two it is what
# `json` reads. A manifest that needs more of JSON5 than that — an unquoted key,
# a single-quoted string — fails here and says why, rather than being read
# wrong.
def strip_json5(src):
    out = []
    i, n = 0, len(src)
    while i < n:
        char = src[i]
        if char in '"\'':
            quote = char
            out.append(char)
            i += 1
            while i < n:
                if src[i] == '\\':
                    out.append(src[i:i + 2])
                    i += 2
                    continue
                out.append(src[i])
                i += 1
                if src[i - 1] == quote:
                    break
            continue
        if src.startswith('//', i):
            while i < n and src[i] != '\n':
                i += 1
            continue
        if src.startswith('/*', i):
            end = src.find('*/', i + 2)
            i = n if end < 0 else end + 2
            continue
        out.append(char)
        i += 1
    return re.sub(r',(\s*[}\]])', r'\1', ''.join(out))


def load_json5(*parts):
    where = '/'.join(parts)
    try:
        return json.loads(strip_json5(read(*parts)))
    except ValueError as error:
        fail('%s does not parse: %s' % (where, error))
        return {}


# --- the manifest ohpm reads ------------------------------------------------

oh = load_json5('oh-package.json5')
for key in ('name', 'version', 'main', 'license'):
    if not oh.get(key):
        fail('oh-package.json5 has no %s, which ohpm requires' % key)

name = oh.get('name', '')
if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', oh.get('version', '')):
    fail('oh-package.json5 version %r is not a version ohpm takes' % oh.get('version'))

main = oh.get('main', '')
if main and not os.path.isfile(path(main)):
    fail('oh-package.json5 main %r is not a file of the module' % main)

# ohpm looks for both of these in the package, and a README of no length is a
# README that was not written.
for file in ('README.md', 'CHANGELOG.md'):
    if not os.path.isfile(path(file)):
        fail('%s is missing: ohpm checks a package for one' % file)
    elif os.path.getsize(path(file)) == 0:
        fail('%s is empty: ohpm checks a package for one' % file)
if not os.path.isfile(os.path.join(root, 'LICENSE')):
    fail('LICENSE is missing at the root: scripts/package_harmony.sh copies it into the HAR')

# The name is what the archive of a release is called, and the two are written
# in two places.
release = os.path.join(root, '.github/workflows/release.yml')
if os.path.isfile(release):
    with open(release) as handle:
        if name and 'dist/%s-*' % name not in handle.read():
            fail('release.yml uploads no dist/%s-*.har: it still names the ohpm name this manifest had' % name)

# --- the manifests hvigor reads ---------------------------------------------

declared = load_json5('src/main/module.json5').get('module', {})
if declared.get('type') != 'har':
    fail('module.json5 is not type "har": %r is not what a HAR carries' % declared.get('type'))
if not declared.get('name'):
    fail('module.json5 names no module')
if not declared.get('deviceTypes'):
    fail('module.json5 lists no deviceTypes, so an app of this module builds for nothing')

profile = load_json5('build-profile.json5')
if profile.get('apiType') != 'stageMode':
    fail('build-profile.json5 apiType %r is not "stageMode"' % profile.get('apiType'))
targets = profile.get('targets') or []
if not targets:
    fail('build-profile.json5 has no targets, so hvigor would have nothing to build')
for target in targets:
    if not target.get('name'):
        fail('build-profile.json5 has a target with no name')
    runtime = target.get('runtimeOS')
    if runtime is not None and runtime != 'HarmonyOS':
        fail('build-profile.json5 target %r is runtimeOS %r, and this package is HarmonyOS'
             % (target.get('name'), runtime))

# --- Index.ets: every export of it is a file of the module ------------------

index = read('Index.ets')
exports = re.findall(r"export\s*\{([^}]*)\}\s*from\s*'([^']+)'", index)
if not exports:
    fail('Index.ets exports nothing, so the package has no surface')
for symbols, target in exports:
    file = os.path.normpath(os.path.join(module, target)) + '.ets'
    if not os.path.isfile(file):
        fail('Index.ets exports from %r and there is no %s' % (target, file))
        continue
    with open(file) as handle:
        source = handle.read()
    for symbol in [symbol.strip() for symbol in symbols.split(',') if symbol.strip()]:
        if not re.search(r'export\s+(?:default\s+)?(?:declare\s+)?'
                         r'(?:abstract\s+)?(?:class|enum|interface|type|const|function|struct)\s+%s\b'
                         % re.escape(symbol), source):
            fail('%s exports no %s, which Index.ets says it does' % (os.path.relpath(file, root), symbol))

# --- the three lists of NAPI names ------------------------------------------

cpp = read('src/main/cpp/napi_init.cpp')
table = re.search(r'napi_property_descriptor\s+\w+\s*\[\s*\]\s*=\s*\{(.*?)\};', cpp, re.S)
if not table:
    fail('napi_init.cpp has no napi_property_descriptor table to check')
    registered = []
else:
    registered = re.findall(r'\{\s*"([A-Za-z_]\w*)"\s*,', table.group(1))
if not registered:
    fail('napi_init.cpp registers no methods')

types = 'src/main/cpp/types/libmarsrs_xlog/index.d.ts'
declarations = re.findall(r'export\s+const\s+([A-Za-z_]\w*)\s*:', read(*types.split('/')))

ets_files = []
for base, _, files in os.walk(path('src/main/ets')):
    for file in files:
        if file.endswith('.ets'):
            ets_files.append(os.path.join(base, file))
if os.path.isfile(path('Index.ets')):
    ets_files.append(path('Index.ets'))
called = set()
for file in ets_files:
    with open(file) as handle:
        for match in re.findall(r'\bxlogNapi\.([A-Za-z_]\w*)\s*\(', handle.read()):
            called.add(match)

for where, names in (('napi_init.cpp registers', registered),
                     ('%s declares' % types, declarations),
                     ('the ArkTS calls', sorted(called))):
    if not names:
        fail('%s nothing' % where)

registered, declarations = set(registered), set(declarations)
for what, missing, here, there in (
        ('declared but not registered', declarations - registered, types, 'napi_init.cpp'),
        ('registered but not declared', registered - declarations, 'napi_init.cpp', types),
        ('called but declared and registered in neither', called - (registered & declarations),
         'the ArkTS', 'napi_init.cpp and %s' % types),
        ('declared and registered but never called', (registered & declarations) - called,
         'napi_init.cpp and %s' % types, 'the ArkTS'),
):
    for method in sorted(missing):
        fail('%s: %s — in %s, not in %s' % (method, what, here, there))

agreed = sorted(registered & declarations & called)

# --- the module's name -------------------------------------------------------

named = re.search(r'\.nm_modname\s*=\s*"([^"]+)"', cpp)
if not named:
    fail('napi_init.cpp sets no nm_modname, so libmarsrs_xlog.so registers no module')
    modname = ''
else:
    modname = named.group(1)

imported = set()
for file in ets_files:
    with open(file) as handle:
        imported.update(re.findall(r"from\s*'lib([A-Za-z_]\w*)\.so'", handle.read()))
if imported != {modname}:
    fail('the ArkTS imports %s and nm_modname is %r: an import of the wrong name resolves no module'
         % (sorted(imported) or 'nothing', modname))

if modname:
    if not os.path.isdir(path('src/main/cpp/types/lib%s' % modname)):
        fail('src/main/cpp/types/lib%s is missing: that is where hvigor looks for the declarations'
             % modname)
    with open(os.path.join(root, 'scripts/build_harmony_napi.sh')) as handle:
        if 'lib%s.so' % modname not in handle.read():
            fail('scripts/build_harmony_napi.sh writes no lib%s.so, which is the library nm_modname names'
                 % modname)

# --- the ArkTS keeps to ArkTS -----------------------------------------------

# Comments and string literals out: what is left is code, and it is the code
# these six words are not allowed to be in.
def code_of(src):
    out = []
    i, n = 0, len(src)
    while i < n:
        char = src[i]
        if char in '"\'`':
            quote = char
            i += 1
            while i < n:
                if src[i] == '\\':
                    i += 2
                    continue
                i += 1
                if src[i - 1] == quote:
                    break
            out.append('""')
            continue
        if src.startswith('//', i):
            while i < n and src[i] != '\n':
                i += 1
            continue
        if src.startswith('/*', i):
            end = src.find('*/', i + 2)
            i = n if end < 0 else end + 2
            continue
        out.append(char)
        i += 1
    return ''.join(out)


# `delete` and `var` are matched as statements and not as words, because
# `Map.delete(key)` is a method and is ArkTS.
forbidden = (
    ('any', re.compile(r'\bany\b')),
    ('unknown', re.compile(r'\bunknown\b')),
    ('var', re.compile(r'(?<![.\w])var\s+[A-Za-z_$]')),
    ('delete', re.compile(r'(?<![.\w])delete\s+[A-Za-z_$]')),
    ('eval', re.compile(r'\beval\s*\(')),
    ('with', re.compile(r'(?<![.\w])with\s*\(')),
)
for file in sorted(ets_files):
    with open(file) as handle:
        source = handle.read()
    code = code_of(source)
    for word, pattern in forbidden:
        for match in pattern.finditer(code):
            line = code.count('\n', 0, match.start()) + 1
            fail('%s:%d is %s, which ArkTS has no room for'
                 % (os.path.relpath(file, root), line, word))

# --- ------------------------------------------------------------------------

if agreed:
    print('%d NAPI methods, the same %d in the C, in the declarations and in the ArkTS:'
          % (len(agreed), len(agreed)))
    print('    %s' % ', '.join(agreed))
print('%d ArkTS file(s) read' % len(ets_files))

if bad:
    for message in bad:
        print('::error::%s' % message)
    sys.exit(1)
print('platforms/harmonyos/marsrs-xlog checks out')
PY
