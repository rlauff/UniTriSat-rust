#!/bin/bash
# Native TSan gate with positive control reversing only the immutable-options
# patch. All sources are instrumented, using the same defines as build.rs.
# Run without concurrent builds/tests/benchmarks. Optional TSAN_RT selects the
# LLVM compiler-rt static runtime; TMPDIR selects the scratch filesystem.
set -euo pipefail
crate=$(cd "$(dirname "$0")/.." && pwd)
rt=${TSAN_RT:-/usr/lib/clang/19/lib/x86_64-redhat-linux-gnu/libclang_rt.tsan.a}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp -a "$crate/vendor/cadical" "$work/racing-source"
patch -R -p1 --fuzz=0 --quiet --batch -d "$work/racing-source" < "$crate/vendor/patches/0002-immutable-options.patch"
defines=(-DNDEBUG -DNBUILD -DNTRACING -DNCONTRIB -DNIPASIR -DNFLEXIBLE -DNUNLOCKED -DNCLOSEFROM -DUNITRISAT_NO_ENVIRONMENT)

for variant in patched racing; do
  src=$crate/vendor/cadical/src
  [[ "$variant" = racing ]] && src=$work/racing-source/src
  objs=$work/$variant-objs
  mkdir -p "$objs"
  for source in "$src"/*.cpp "$crate/tsan/driver.cpp"; do
    g++ -std=c++17 -O1 -g -fsanitize=thread -pthread "${defines[@]}" -I"$src" \
      -c "$source" -o "$objs/$(basename "$source" .cpp).o"
  done
  gcc -std=c11 -O1 -g -fsanitize=thread "${defines[@]}" -I"$src" \
    -c "$src/kitten.c" -o "$objs/kitten.o"
  g++ -o "$work/$variant" "$objs"/*.o -Wl,--whole-archive "$rt" \
    -Wl,--no-whole-archive -lm -lpthread -ldl -lrt
  status=0
  TSAN_OPTIONS=halt_on_error=0:exitcode=66 "$work/$variant" > "$work/$variant.out" 2>&1 || status=$?
  cat "$work/$variant.out"
  if [[ $(grep -c 'wrong results' "$work/$variant.out" || true) != 1 ]] ||
     ! grep -qx 'wrong results: 0' "$work/$variant.out" ||
     grep -Eq '(FATAL:|ERROR:|CHECK failed:|DEADLYSIGNAL)' "$work/$variant.out"; then
    echo "$variant: unsuccessful or missing completion" >&2
    exit 1
  fi
  if [[ "$variant" = patched ]]; then
    if [[ "$status" != 0 ]] || grep -Eq '(WARNING:|SUMMARY:)' "$work/$variant.out"; then
      echo 'patched: expected clean exit and no sanitizer reports' >&2
      exit 1
    fi
  else
    if [[ "$status" != 66 ]] ||
       ! grep -q '^WARNING: ThreadSanitizer: data race ' "$work/$variant.out" ||
       ! grep -Eq '^SUMMARY: ThreadSanitizer: data race .*options\.cpp:[0-9]+' "$work/$variant.out" ||
       awk '/^WARNING:/ && !/^WARNING: ThreadSanitizer: data race / {bad=1} END {exit !bad}' "$work/$variant.out"; then
      echo 'racing: expected Options constructor data race and exit 66' >&2
      exit 1
    fi
  fi
  echo "== $variant: accepted (exit $status)"
done
