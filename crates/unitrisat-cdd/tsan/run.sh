#!/bin/bash
# ThreadSanitizer check of the vendored cddlib as this crate uses it (task P1.9):
# driver.c initializes cddlib once, then 8 threads compute hulls and solve LPs
# concurrently on their own objects through shim/ucdd.c, without a lock.
#
# Builds the driver twice, against the patched vendor tree and against the
# pristine one (the patch reversed in a temporary copy), and runs both.
# Expected: pristine reports data races on the LP pivot counters only;
# patched reports none. Both must print "wrong results: 0".
#
# usage: tsan/run.sh <GMP prefix with include/gmp.h and lib/libgmp.a>
#   e.g. tsan/run.sh ../../target/release/build/gmp-mpfr-sys-*/out
#
# GCC instruments the code (-fsanitize=thread at compile time). The TSan
# runtime is LLVM compiler-rt's static library, linked by hand, because
# Fedora's GCC libtsan package is not installed here; set TSAN_RT to use
# another one.
set -euo pipefail

# Also usable to recheck saved logs without compiling or running native code.
check_output() { # <patched|pristine> <process status> <output file>
  local variant=$1 status=$2 output=$3
  if [[ "$variant" != patched && "$variant" != pristine ]] ||
     [[ ! "$status" =~ ^[0-9]+$ ]] || [[ ! -f "$output" ]]; then
    echo "invalid sanitizer result arguments" >&2
    return 1
  fi
  # A missing, duplicate, malformed or nonzero completion record is failure.
  if [[ $(grep -c 'wrong results' "$output" || true) != 1 ]] ||
     ! grep -qx 'wrong results: 0' "$output"; then
    echo "$variant: missing or unsuccessful completion record" >&2
    return 1
  fi
  if grep -Eq '(FATAL:|ERROR:|CHECK failed:|DEADLYSIGNAL)' "$output"; then
    echo "$variant: sanitizer/runtime failure" >&2
    return 1
  fi
  if [[ "$variant" = patched ]]; then
    if [[ "$status" != 0 ]] || grep -Eq '(WARNING:|SUMMARY:)' "$output"; then
      echo "patched: expected clean exit and no sanitizer reports" >&2
      return 1
    fi
  else
    # TSan's exit status is fixed by the runner. The unpatched source is the
    # positive control: a successful race-free run proves no sensitivity.
    if [[ "$status" != 66 ]] ||
       ! grep -q '^WARNING: ThreadSanitizer: data race ' "$output" ||
       ! grep -Eq '^SUMMARY: ThreadSanitizer: data race .*cddlp(_f)?\.c:[0-9]+' "$output" ||
       awk '/^WARNING:/ && !/^WARNING: ThreadSanitizer: data race / {bad=1} END {exit !bad}' "$output"; then
      echo "pristine: expected recognized cddlib data race and exit 66" >&2
      return 1
    fi
  fi
}

if [[ ${1:-} = --check-output ]]; then
  [[ $# = 4 ]] || { echo 'usage: --check-output variant status output' >&2; exit 1; }
  check_output "$2" "$3" "$4"
  exit
fi
[[ $# = 1 ]] || { echo 'usage: tsan/run.sh GMP_PREFIX' >&2; exit 1; }
crate=$(cd "$(dirname "$0")/.." && pwd)
gmp=$(cd "$1" && pwd)
rt=${TSAN_RT:-/usr/lib/clang/19/lib/x86_64-redhat-linux-gnu/libclang_rt.tsan.a}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cp -a "$crate/vendor" "$work/pristine-vendor"
(cd "$work/pristine-vendor" && patch -R -p1 --fuzz=0 --quiet < patches/0001-thread-local-pivot-counters.patch)

build() { # <vendor dir> <binary>
  local vendor=$1 objs=$work/objs-$(basename "$2")
  mkdir -p "$objs"
  for f in "$vendor"/cddlib/lib-src/{cddcore,cddlp,cddmp,cddio,cddlib,cddproj,setoper}.c \
           "$vendor"/cddlib/generated/{cddcore_f,cddlp_f,cddmp_f,cddio_f,cddlib_f,cddproj_f}.c \
           "$crate/shim/ucdd.c" "$crate/tsan/driver.c"; do
    gcc -std=c11 -O1 -g -fsanitize=thread -DGMPRATIONAL -w -I"$gmp/include" \
      -I"$vendor/cddlib/lib-src" -I"$vendor/cddlib/generated" -c "$f" -o "$objs/$(basename "$f" .c).o"
  done
  gcc -o "$2" "$objs"/*.o "$gmp/lib/libgmp.a" -Wl,--whole-archive "$rt" -Wl,--no-whole-archive \
    -lm -lpthread -ldl -lrt -lstdc++
}

for variant in patched pristine; do
  vendor=$crate/vendor
  [ "$variant" = pristine ] && vendor=$work/pristine-vendor
  build "$vendor" "$work/$variant"
  status=0
  TSAN_OPTIONS=halt_on_error=0:exitcode=66 "$work/$variant" > "$work/$variant.out" 2>&1 || status=$?
  echo "== $variant: exit $status, $(grep -c 'WARNING: ThreadSanitizer' "$work/$variant.out" || true) TSan warnings; $(grep 'wrong results' "$work/$variant.out")"
  check_output "$variant" "$status" "$work/$variant.out" || { cat "$work/$variant.out" >&2; exit 1; }
  grep '^SUMMARY' "$work/$variant.out" | sed 's/.*vendor\///; s/.*pristine-vendor\///' | sort | uniq -c || true
done
