#!/bin/bash
# Regenerate target.txt: the php-src tests real PHP passes.
#
# Runs php-src's own run-tests.php with the php:8.4-cli image (PHP 8.4.27)
# over a php-src checkout at the same version, and keeps every test case
# that passed (no failure, error or skip in its JUnit entry).
#
#   tests/php-src/make-target.sh /path/to/php-src   (checked out at php-8.4.27)
set -euo pipefail
src=${1:?usage: make-target.sh /path/to/php-src}
here=$(cd "$(dirname "$0")" && pwd)
junit=$(mktemp "${TMPDIR:-.}/junit.XXXXXX")
tar --exclude=.git -C "$src" -cf - . | docker run --rm -i php:8.4-cli bash -c '
  mkdir -p /src && cd /src && tar xf - 2>/dev/null
  export TEST_PHP_EXECUTABLE=$(which php) NO_INTERACTION=1 SKIP_SLOW_TESTS=1 TEST_PHP_JUNIT=/tmp/junit.xml
  php run-tests.php -j8 -q --no-color -d display_errors=1 Zend tests ext sapi > /dev/null 2>&1 || true
  cat /tmp/junit.xml' > "$junit"
python3 - "$junit" "$here/target.txt" <<'PY'
import re, sys
xml = open(sys.argv[1], encoding="utf-8", errors="replace").read()
cases = re.findall(r"<testcase name='(.*?) \(.*?' time='[^']*'>(.*?)</testcase>", xml, re.S)
passed = sorted({name for name, body in cases if not re.search(r"<(failure|error|skipped)", body)})
open(sys.argv[2], "w").write("\n".join(passed) + "\n")
print(f"{len(passed)} tests pass with real PHP")
PY
rm -f "$junit"
