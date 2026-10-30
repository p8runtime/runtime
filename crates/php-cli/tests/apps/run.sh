#!/bin/bash
# An app's own test suite under PHP 8.4 (docker, ./Dockerfile) and under p8,
# compared test by test (./compare.php): how many of the tests PHP passes
# p8 passes too. The app is described by ./<app>.conf (its repository and
# pinned commit, the test command, how tests are grouped). Writes the app
# and its suite as p8-test-suite lines to .tmp/apps/<app>/suite.jsonl.
# Exit 0 when both runs completed.
#
# The test dependencies are pinned: the app's own composer.lock, or
# ./locks/<app>.lock (made on the first run, committed with the .conf).
# Both runs use the checkout's tracked files exactly as pinned; SETUP (a
# shell command, under PHP) makes what the tests need, such as an .env,
# and the files named in SETUP_FILES come back for p8's run.
#
# Usage: crates/php-cli/tests/apps/run.sh <app> [path to php2]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
app=$1
php2=${2:-$repo/target/release/php2}
conf="$here/$app.conf"
[ -f "$conf" ] || { echo "$conf: no such app"; exit 1; }
SETUP='' SETUP_FILES='' TIMEOUT=14400
# shellcheck source=/dev/null
. "$conf"
work=$repo/.tmp/apps/$app
mkdir -p "$work"
cd "$work"

# The code at the pinned commit.
if [ "$(git -C src rev-parse HEAD 2>/dev/null || true)" != "$COMMIT" ]; then
    [ -d src ] && find src -delete
    git init -q src
    git -C src fetch -q --depth 1 "$REPO" "$COMMIT"
    git -C src checkout -q FETCH_HEAD
fi
cd src
git checkout -q -- .

image=p8-php-apps:$(shasum -a 256 "$here/Dockerfile" | cut -c1-12)
docker image inspect "$image" >/dev/null 2>&1 || docker build -q -t "$image" "$here" >/dev/null
top=/$(printf '%s' "$PWD" | cut -d/ -f2)
# A shell command under PHP in a copy of the checkout at the same path (the
# tests print paths); what it writes to stdout comes back.
in_php() {
    COPYFILE_DISABLE=1 tar --no-xattrs -cf - . 2>/dev/null |
        docker run --rm -i --user "$(id -u):$(id -g)" -e HOME=/tmp -e COMPOSER_HOME=/tmp/c --tmpfs "${top}:uid=$(id -u),gid=$(id -g),exec,size=8g" "$image" \
            bash -c "mkdir -p '$PWD' && cd '$PWD' && tar xf - 2>/dev/null && $1"
}

# The test dependencies, from the pinned lock.
pinned="$here/locks/$app.lock"
if git ls-files --error-unmatch composer.lock >/dev/null 2>&1; then
    lockfile=$PWD/composer.lock
else
    lockfile=$pinned
fi
if [ ! -f "$lockfile" ]; then
    # First run: resolve the dependencies once and pin them.
    mkdir -p "$here/locks"
    in_php "curl -sS https://getcomposer.org/installer | php -- --quiet --install-dir=/tmp && php /tmp/composer.phar update -q --no-progress --no-scripts --ignore-platform-reqs >&2 && cat composer.lock" > "$pinned"
fi
lock=$(shasum -a 256 "$lockfile" | cut -d' ' -f1)
if [ "$(cat vendor/.p8-lock 2>/dev/null || true)" != "$lock" ]; then
    [ -d vendor ] && find vendor -delete
    cp "$lockfile" composer.lock
    in_php "curl -sS https://getcomposer.org/installer | php -- --quiet --install-dir=/tmp && php /tmp/composer.phar install -q --no-progress --no-scripts --ignore-platform-reqs >&2 && tar cf - vendor" | tar xf -
    echo "$lock" > vendor/.p8-lock
fi
git checkout -q -- .
if [ -n "$SETUP" ]; then
    # shellcheck disable=SC2086
    in_php "$SETUP >&2 && tar cf - $SETUP_FILES" | tar xf -
fi

# PHP.
in_php "timeout $TIMEOUT php -d memory_limit=4G $TEST_CMD --log-junit /tmp/php.xml > /tmp/php.txt 2>&1; tar cf - -C /tmp php.xml php.txt" | tar xf - -C "$work"
tail -3 "$work/php.txt"

# p8.
p8_status=0
timeout "$TIMEOUT" "$php2" -d memory_limit=4G $TEST_CMD --log-junit "$work/p8.xml" --log-teamcity "$work/p8.tc" > "$work/p8.txt" 2>&1 || p8_status=$?
tail -3 "$work/p8.txt"

php_version=$(docker run --rm "$image" php -r 'echo PHP_VERSION;')
# suite.jsonl: the app, then the comparison's suite line; its summary to
# the terminal.
printf '{"type":"app","name":%s,"version":%s,"kind":%s,"about":%s,"source":%s}\n' \
    "$(printf '%s' "$NAME" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')" "\"$VERSION\"" "\"$KIND\"" \
    "$(printf '%s' "$ABOUT" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')" "\"$SOURCE\"" > "$work/suite.jsonl"
COPYFILE_DISABLE=1 tar --no-xattrs -cf - -C "$here" compare.php -C "$work" php.xml p8.tc |
    docker run --rm -i "$image" bash -c "mkdir /c && cd /c && tar xf - && php compare.php php.xml p8.tc suite.jsonl '$(git -C "$repo" rev-parse HEAD)' '$php_version' \
        '$NAME' '$VERSION' \"$SUITE_NAME\" '$SUITE_SOURCE' '$TESTS_ROOT' >&2 && cat suite.jsonl" \
    >> "$work/suite.jsonl"
# PHPUnit exits 1 or 2 when tests fail, and ends with its summary; a run
# that stopped (a crash, a timeout) has none: its tests not run count as
# not passing.
grep -qE '^(Tests: |OK \(|  Tests: )' "$work/p8.txt" || echo "the p8 run stopped (exit $p8_status): tests it did not reach count as not passing"
