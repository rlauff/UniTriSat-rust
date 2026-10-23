"""Reproduce cddlib's generated floating sources (`*_f.c`, `*_f.h`).

Mechanical port of the `SED_GMP` recipe in cddlib `lib-src/Makefile.gmp.am`
lines 29-44 (commit 86dfe60811ec879a46e73d488719acaa754536e4):

    sed -e 's/dd_/ddf_/g' -e 's/cddf_/cdd_/g' -e 's/mytype/myfloat/g' \
        -e 's/#include "cdd.h"/#include "cdd_f.h"/' ... -e 's/CDOUBLE/ddf_CDOUBLE/g' \
        $< | awk 'BEGIN{print "/* generated automatically from $< */"}1' > $@

Semantics reproduced exactly:
- The input is processed line by line. A line is the bytes up to a LF, and the
  LF is not part of the pattern space.
- The 13 `s` commands run in the listed order on each line. Commands marked
  `/g` replace every non-overlapping leftmost match. All other commands replace
  only the first (leftmost) match on the line.
- Patterns are POSIX basic regular expressions. In these 13 patterns the only
  metacharacter is `.`, which matches any single character. For example,
  `#include "cdd.h"` would also match `#include "cddXh"`. This port keeps that
  behavior: every `.` becomes a one-byte wildcard and every other byte is
  literal. It is exact under the C locale. The pinned inputs are pure ASCII, so
  locale cannot matter for them, and the script rejects non-ASCII input rather
  than guess.
- awk prints a first line, `/* generated automatically from <prereq> */`, then
  every input line followed by LF. That adds a final LF if the input lacks one.
  `<prereq>` is make's `$<` for an in-tree build, which is the bare source file
  name, for example `cddcore.c`.

Usage (stdlib only, no shell, no compiler):
    python3 -I -B generate_float_sources.py LIB_SRC_DIR OUT_DIR

OUT_DIR must not exist; it is created. Prints one JSON receipt (input and output
sha256, plus per-rule match counts) on stdout. Exits nonzero on any error.
"""
import hashlib
import json
import os
import re
import sys

# (pattern, replacement, global) exactly in upstream order.
RULES = (
    ('dd_', 'ddf_', True),
    ('cddf_', 'cdd_', True),
    ('mytype', 'myfloat', True),
    ('#include "cdd.h"', '#include "cdd_f.h"', False),
    ('#include "cddtypes.h"', '#include "cddtypes_f.h"', False),
    ('#include "cddmp.h"', '#include "cddmp_f.h"', False),
    ('__CDD_H', '__CDD_HF', False),
    ('__CDD_HFF', '__CDD_HF', False),
    ('__CDDMP_H', '_CDDMP_HF', False),
    ('__CDDTYPES_H', '_CDDTYPES_HF', False),
    ('GMPRATIONAL', 'ddf_GMPRATIONAL', True),
    ('ARITHMETIC', 'ddf_ARITHMETIC', True),
    ('CDOUBLE', 'ddf_CDOUBLE', True),
)

C_UNITS = ('cddcore', 'cddlp', 'cddmp', 'cddio', 'cddlib', 'cddproj')
HEADERS = ('cdd', 'cddmp', 'cddtypes')


def bre(pattern):
    """BRE with `.` as the only metacharacter -> bytes regex."""
    return re.compile(b''.join(b'.' if ch == '.' else re.escape(ch.encode('ascii'))
                               for ch in pattern))


COMPILED = tuple((bre(p), r.encode('ascii'), g, p) for p, r, g in RULES)


def transform(data, prereq, counts, wildcard_hits):
    if any(byte > 127 for byte in data):
        raise ValueError(prereq + ': non-ASCII input; locale semantics not reproduced')
    lines = data.split(b'\n')
    if lines and lines[-1] == b'':
        lines.pop()                     # trailing LF terminates the last line
    out = [b'/* generated automatically from ' + prereq.encode('ascii') + b' */\n']
    for line in lines:
        for regex, replacement, is_global, source in COMPILED:
            for match in regex.finditer(line) if is_global else filter(None, [regex.search(line)]):
                counts[source] = counts.get(source, 0) + 1
                if '.' in source and match.group(0) != source.encode('ascii'):
                    wildcard_hits.append([prereq, source, match.group(0).decode('ascii')])
            line = regex.sub(lambda _, r=replacement: r, line, count=0 if is_global else 1)
        out.append(line + b'\n')
    return b''.join(out)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main(argv):
    if len(argv) != 3:
        raise ValueError('usage: generate_float_sources.py LIB_SRC_DIR OUT_DIR')
    src, dst = argv[1], argv[2]
    os.mkdir(dst)                       # fails if OUT_DIR already exists
    receipt = {'inputs': {}, 'outputs': {}, 'rule_match_counts': {}, 'nonliteral_wildcard_matches': []}
    jobs = [(n + '.c', n + '_f.c') for n in C_UNITS] + [(n + '.h', n + '_f.h') for n in HEADERS]
    for name, generated in jobs:
        with open(os.path.join(src, name), 'rb') as handle:
            data = handle.read()
        counts = {}
        result = transform(data, name, counts, receipt['nonliteral_wildcard_matches'])
        with open(os.path.join(dst, generated), 'xb') as handle:   # create_new
            handle.write(result)
        receipt['inputs'][name] = sha(data)
        receipt['outputs'][generated] = sha(result)
        receipt['rule_match_counts'][generated] = [counts.get(p, 0) for p, _, _ in RULES]
    receipt['rules'] = [[p, r, 'g' if g else 'first'] for p, r, g in RULES]
    print(json.dumps(receipt, indent=1, sort_keys=True))


if __name__ == '__main__':
    try:
        main(sys.argv)
    except (OSError, ValueError) as error:
        print('generation failed: ' + str(error), file=sys.stderr)
        sys.exit(1)
