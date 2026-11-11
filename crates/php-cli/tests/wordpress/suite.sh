#!/bin/bash
# WordPress's own PHPUnit tests (wordpress-develop, pinned) under the
# reference PHP (tests/php-src/reference.sh: docker, with the extensions
# WordPress's CI has) and under p8, on
# MariaDB, compared test by test (../apps/compare.php): how many of the tests
# PHP passes p8 passes too. Writes the run as a p8-test-suite "suite" line
# to .tmp/wordpress-suite/suite.jsonl. Exit 0 when both runs completed.
#
# Pinned: wordpress-develop's commit for the release, the release's build
# (its tarball, sha1-checked: the tests want built files), and the test
# dependencies (suite/composer.lock; WordPress ships none).
#
# Both runs use the tests exactly as pinned: any local change to tracked
# files is undone first.
#
# Usage: crates/php-cli/tests/wordpress/suite.sh [path to p8]
# Needs docker and the p8-test-suite checkout next to this repository (or
# P8_TEST_SUITE=<its path>), for the PHP version it pins. Takes ~10
# minutes under PHP, longer under p8.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
p8=${1:-$repo/target/release/p8}
work=$repo/.tmp/wordpress-suite
version=7.1.4
commit=0afb74de8c3fe4c1869e44b62b0418d67b11aa4c
mkdir -p "$work"
cd "$work"

# The tests at the release's commit.
if [ "$(git -C wpdev rev-parse HEAD 2>/dev/null || true)" != "$commit" ]; then
    [ -d wpdev ] && find wpdev -delete
    git init -q wpdev
    git -C wpdev fetch -q --depth 1 https://github.com/WordPress/wordpress-develop "$commit"
    git -C wpdev checkout -q FETCH_HEAD
fi
cd wpdev
# The release's build, which the tests run against.
if [ "$(sed -n "s/^\$wp_version = '\(.*\)';/\1/p" build/wp-includes/version.php 2>/dev/null || true)" != "$version" ]; then
    [ -d build ] && find build -delete
    curl -sSfL -o ../wp.tar.gz "https://wordpress.org/wordpress-$version.tar.gz"
    curl -sSfL -o ../wp.tar.gz.sha1 "https://wordpress.org/wordpress-$version.tar.gz.sha1"
    [ "$(shasum -a 1 ../wp.tar.gz | cut -d' ' -f1)" = "$(cat ../wp.tar.gz.sha1)" ] || { echo "wordpress-$version.tar.gz: sha1 mismatch"; exit 1; }
    mkdir build
    tar xzf ../wp.tar.gz -C build --strip-components 1
fi
# The test dependencies, from the pinned lock.
lock=$(shasum -a 256 "$here/suite/composer.lock" | cut -d' ' -f1)
if [ "$(cat vendor/.p8-lock 2>/dev/null || true)" != "$lock" ]; then
    [ -d vendor ] && find vendor -delete
    # WordPress turns Composer's lock off: on, then the pinned lock.
    COPYFILE_DISABLE=1 tar --no-xattrs -cf - composer.json -C "$here/suite" composer.lock |
        docker run --rm -i -u "$(id -u):$(id -g)" -e COMPOSER_HOME=/tmp/c --tmpfs "/w:uid=$(id -u),gid=$(id -g),exec" composer:2 \
            sh -c 'mkdir /w/in && cd /w/in && tar xf - && cd /w && mv in/composer.json . && composer config lock true && mv in/composer.lock . && composer install -q --no-progress --no-scripts --ignore-platform-reqs >&2 && tar cf - vendor' |
        tar xf -
    echo "$lock" > vendor/.p8-lock
fi
sed -e "s#dirname( __FILE__ ) . '/src/'#dirname( __FILE__ ) . '/build/'#" -e "s/'youremptytestdbnamehere'/getenv('WPDBNAME')/" \
    -e "s/'yourusernamehere'/'root'/" -e "s/'yourpasswordhere'/'p8root'/" -e "s/'localhost'/getenv('WPDBHOST')/" \
    -e "s#define( 'WP_PHP_BINARY', 'php' );#define( 'WP_PHP_BINARY', getenv('WPPHP') ?: 'php' );#" wp-tests-config-sample.php > wp-tests-config.php
git checkout -q -- .

docker network inspect p8net >/dev/null 2>&1 || docker network create p8net >/dev/null
if ! docker ps --format '{{.Names}}' | grep -qx p8-mariadb; then
    docker rm -f p8-mariadb >/dev/null 2>&1 || true
    docker run -d --name p8-mariadb --network p8net -p 127.0.0.1:3307:3306 \
        -e MARIADB_ROOT_PASSWORD=p8root -e MARIADB_USER=wp -e MARIADB_PASSWORD=wp mariadb:11.4 >/dev/null
    until docker exec p8-mariadb mariadb -uroot -pp8root -e 'SELECT 1' >/dev/null 2>&1; do sleep 1; done
fi
# shellcheck source=../../../../tests/php-src/reference.sh
. "$repo/tests/php-src/reference.sh"
php_reference "${P8_TEST_SUITE:-$repo/../p8-test-suite}"
image=$(php_image p8-php-wptests "$here/suite/Dockerfile" "$here/suite")
for db in wptests_php wptests_p8; do
    docker exec p8-mariadb mariadb -uroot -pp8root -e "DROP DATABASE IF EXISTS $db; CREATE DATABASE $db;"
done

# PHP, at the same path (the tests print paths).
top=/$(printf '%s' "$PWD" | cut -d/ -f2)
COPYFILE_DISABLE=1 tar --no-xattrs -cf - . 2>/dev/null |
    docker run --rm -i --network p8net --user "$(id -u):$(id -g)" --tmpfs "$top:uid=$(id -u),gid=$(id -g),exec,size=4g" \
        -e WPDBHOST=p8-mariadb -e WPDBNAME=wptests_php "$image" \
        bash -c "mkdir -p '$PWD' && cd '$PWD' && tar xf - 2>/dev/null && php -d memory_limit=1G vendor/bin/phpunit --log-junit /tmp/php.xml > /tmp/php.txt 2>&1; tar cf - -C /tmp php.xml php.txt" |
    tar xf - -C "$work"
tail -3 "$work/php.txt"

# p8.
p8_status=0
WPDBHOST=127.0.0.1:3307 WPDBNAME=wptests_p8 WPPHP="$p8" \
    "$p8" -d memory_limit=1G vendor/bin/phpunit --log-junit "$work/p8.xml" --log-teamcity "$work/p8.tc" > "$work/p8.txt" 2>&1 || p8_status=$?
tail -3 "$work/p8.txt"

php_version=$(docker run --rm "$image" php -r 'echo PHP_VERSION;')
# The comparison's summary to the terminal, its suite line to suite.jsonl.
COPYFILE_DISABLE=1 tar --no-xattrs -cf - -C "$here/../apps" compare.php -C "$work" php.xml p8.tc |
    docker run --rm -i "$image" bash -c "mkdir /c && cd /c && tar xf - && php compare.php php.xml p8.tc suite.jsonl '$(git -C "$repo" rev-parse HEAD)' '$php_version' \
        WordPress '$version' \"WordPress's PHPUnit tests\" 'https://github.com/WordPress/wordpress-develop/tree/$version/tests/phpunit' tests/phpunit/tests/ >&2 && cat suite.jsonl" \
    > "$work/suite.jsonl"
# PHPUnit exits 1 or 2 when tests fail, and ends with its summary; a run
# that stopped (a crash, or PHPUnit's own error exits 2 too) has none.
{ [ "$p8_status" -le 2 ] && grep -qE '^(Tests: |OK \()' "$work/p8.txt"; } || { echo "the p8 run stopped (exit $p8_status)"; exit 1; }
