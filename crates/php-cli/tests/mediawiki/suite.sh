#!/bin/bash
# MediaWiki's own PHPUnit unit tests (the core:unit suite, as MediaWiki's CI
# runs it: `composer phpunit:unit`) under the reference PHP
# (tests/php-src/reference.sh: docker, with the
# extensions MediaWiki requires) and under p8, compared test by test
# (../apps/compare.php): how many of the tests PHP passes p8 passes too.
# Writes the run as a p8-test-suite "suite" line to
# .tmp/mediawiki-suite/suite.jsonl. Exit 0 when both runs completed.
#
# Pinned: MediaWiki's commit for the release and the test dependencies
# (suite/composer.lock; MediaWiki ships none). The wiki is installed once
# under PHP (SQLite) for its LocalSettings.php: the unit bootstrap loads the
# settings, and unit tests touch no database. Both runs use the checkout's
# tracked files exactly as pinned.
#
# Usage: crates/php-cli/tests/mediawiki/suite.sh [path to p8]
# Needs docker and the p8-test-suite checkout next to this repository (or
# P8_TEST_SUITE=<its path>), for the PHP version it pins. Takes about a
# minute under PHP, longer under p8.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
p8=${1:-$repo/target/release/p8}
work=$repo/.tmp/mediawiki-suite
version=1.46.2
commit=d0d9a283f219a1c9487754dc96598080a4a4e30a
mkdir -p "$work/db"
cd "$work"

# The code at the release's commit.
if [ "$(git -C mw rev-parse HEAD 2>/dev/null || true)" != "$commit" ]; then
    [ -d mw ] && find mw -delete
    git init -q mw
    git -C mw fetch -q --depth 1 https://github.com/wikimedia/mediawiki "$commit"
    git -C mw checkout -q FETCH_HEAD
fi
cd mw
# The test dependencies, from the pinned lock.
lock=$(shasum -a 256 "$here/suite/composer.lock" | cut -d' ' -f1)
if [ "$(cat vendor/.p8-lock 2>/dev/null || true)" != "$lock" ]; then
    [ -d vendor ] && find vendor -delete
    COPYFILE_DISABLE=1 tar --no-xattrs -cf - composer.json -C "$here/suite" composer.lock |
        docker run --rm -i -u "$(id -u):$(id -g)" -e COMPOSER_HOME=/tmp/c --tmpfs "/w:uid=$(id -u),gid=$(id -g),exec" composer:2 \
            sh -c 'cd /w && tar xf - && composer install -q --no-progress --no-scripts --ignore-platform-reqs >&2 && tar cf - vendor' |
        tar xf -
    echo "$lock" > vendor/.p8-lock
fi
git checkout -q -- .

# shellcheck source=../../../../tests/php-src/reference.sh
. "$repo/tests/php-src/reference.sh"
php_reference "${P8_TEST_SUITE:-$repo/../p8-test-suite}"
image=$(php_image p8-php-mwtests "$here/suite/Dockerfile" "$here/suite")
top=/$(printf '%s' "$PWD" | cut -d/ -f2)
# Runs a shell command under PHP in a copy of the checkout at the same path
# (the tests print paths); files it names on stdout come back.
in_php() {
    COPYFILE_DISABLE=1 tar --no-xattrs -cf - . 2>/dev/null |
        docker run --rm -i --user "$(id -u):$(id -g)" --tmpfs "${top}:uid=$(id -u),gid=$(id -g),exec,size=4g" "$image" \
            bash -c "mkdir -p '$PWD' '$work/db' && cd '$PWD' && tar xf - 2>/dev/null && $1"
}
# The wiki's settings, once (an earlier one in the checkout stops the installer).
if [ ! -f "$work/LocalSettings.php" ]; then
    [ -f LocalSettings.php ] && find LocalSettings.php -delete
    in_php "php maintenance/run.php install --dbtype sqlite --dbpath '$work/db' --server http://localhost --scriptpath '' --pass p8p8p8p8p8p8 p8wiki Admin >&2 && cat LocalSettings.php" > "$work/LocalSettings.php.new"
    # (in place only once the install succeeded)
    mv "$work/LocalSettings.php.new" "$work/LocalSettings.php"
fi
cp "$work/LocalSettings.php" LocalSettings.php
# The PHPUnit configuration MediaWiki generates (composer phpunit:config).
in_php "php tests/phpunit/generatePHPUnitConfig.php >&2 && cat phpunit.xml" > phpunit.xml

# PHP.
in_php "php -d memory_limit=2G vendor/bin/phpunit --testsuite core:unit --log-junit /tmp/php.xml > /tmp/php.txt 2>&1; tar cf - -C /tmp php.xml php.txt" |
    tar xf - -C "$work"
tail -3 "$work/php.txt"

# p8.
p8_status=0
"$p8" -d memory_limit=2G vendor/bin/phpunit --testsuite core:unit --log-junit "$work/p8.xml" --log-teamcity "$work/p8.tc" > "$work/p8.txt" 2>&1 || p8_status=$?
tail -3 "$work/p8.txt"

php_version=$(docker run --rm "$image" php -r 'echo PHP_VERSION;')
# suite.jsonl: the app (no page gate declares MediaWiki), then the
# comparison's suite line; its summary to the terminal.
printf '%s\n' '{"type":"app","name":"MediaWiki","version":"'"$version"'","kind":"application","about":"The wiki engine behind Wikipedia.","source":"https://www.mediawiki.org"}' > "$work/suite.jsonl"
COPYFILE_DISABLE=1 tar --no-xattrs -cf - -C "$here/../apps" compare.php -C "$work" php.xml p8.tc |
    docker run --rm -i "$image" bash -c "mkdir /c && cd /c && tar xf - && php compare.php php.xml p8.tc suite.jsonl '$(git -C "$repo" rev-parse HEAD)' '$php_version' \
        MediaWiki '$version' \"MediaWiki's PHPUnit unit tests\" 'https://github.com/wikimedia/mediawiki/tree/$version/tests/phpunit/unit' \
        tests/phpunit/unit/includes/,tests/phpunit/unit/ >&2 && cat suite.jsonl" \
    >> "$work/suite.jsonl"
# PHPUnit exits 1 or 2 when tests fail, and ends with its summary; a run
# that stopped (a crash, or PHPUnit's own error exits 2 too) has none.
{ [ "$p8_status" -le 2 ] && grep -qE '^(Tests: |OK \()' "$work/p8.txt"; } || { echo "the p8 run stopped (exit $p8_status)"; exit 1; }
