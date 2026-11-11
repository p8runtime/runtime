#!/bin/bash
# Regenerate p8-test-suite's target.txt: the php-src tests real PHP passes.
#
# Builds the real PHP from the release the suite pins (php-src.json: tag and
# the SHA-256 of its GitHub archive) with php-src.Dockerfile, the official
# php:<version>-cli recipe plus mbstring. Then runs php-src's own
# run-tests.php with it over a php-src checkout at the same tag (in parallel,
# then the failures again one at a time), and keeps every test case that
# passed (no failure, error or skip in its JUnit entry).
#
#   crates/php-test/php-src/make-target.sh /path/to/php-src /path/to/p8-test-suite
#   (php-src checked out at the suite's php-src.json commit)
set -euo pipefail
src=${1:?usage: make-target.sh /path/to/php-src /path/to/p8-test-suite}
suite=${2:?usage: make-target.sh /path/to/php-src /path/to/p8-test-suite}
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=reference.sh
. "$here/reference.sh"
php_reference "$suite"
image=$php_ref
junit=$(mktemp "${TMPDIR:-.}/junit.XXXXXX")
tar --exclude=.git -C "$src" -cf - . | docker run --rm -i "$image" bash -c '
  mkdir -p /src && cd /src && tar xf - 2>/dev/null
  export TEST_PHP_EXECUTABLE=$(which php) NO_INTERACTION=1 SKIP_SLOW_TESTS=1
  TEST_PHP_JUNIT=/tmp/junit.xml php run-tests.php -j"$(nproc)" -q --no-color -d display_errors=1 -w /tmp/failed.txt Zend tests ext sapi > /dev/null 2>&1 || true
  # Tests that failed next to others (shared files, memory, timeouts) get a
  # second, serial run; one that passes alone is a test real PHP passes.
  : > /tmp/junit2.xml
  [ -s /tmp/failed.txt ] && { TEST_PHP_JUNIT=/tmp/junit2.xml php run-tests.php -j1 -q --no-color -d display_errors=1 -r /tmp/failed.txt > /dev/null 2>&1 || true; }
  cat /tmp/junit.xml /tmp/junit2.xml' > "$junit"
python3 - "$junit" "$suite/target.txt" <<'PY'
import re, sys
# Both runs' JUnit files, one after the other: a test counts when any of its
# entries has no failure, error or skip.
xml = open(sys.argv[1], encoding="utf-8", errors="replace").read()
cases = re.findall(r"<testcase name='(.*?) \(.*?' time='[^']*'>(.*?)</testcase>", xml, re.S)
passed = sorted({name for name, body in cases if not re.search(r"<(failure|error|skipped)", body)})
open(sys.argv[2], "w").write("\n".join(passed) + "\n")
print(f"{len(passed)} tests pass with real PHP")
PY
rm -f "$junit"
