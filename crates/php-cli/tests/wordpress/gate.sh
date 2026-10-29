#!/bin/bash
# WordPress gate: install WordPress on MariaDB under PHP 8.4 (docker,
# with mysqli) and under p8, then request the same pages from each: the
# public ones (home, a post, the feed, the REST API root, a search, the
# login form) and, signed in as the admin, wp-admin's (dashboard, posts,
# new post, settings, users, plugins). Every page must be byte-identical,
# apart from the clock readings wp-admin prints. Exit 0 when they all are.
#
# Usage: crates/php-cli/tests/wordpress/gate.sh [path to php2]
# Needs docker. Works in .tmp/wordpress-gate; MariaDB listens on
# 127.0.0.1:3307 (container p8-mariadb, network p8net).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
php2=${1:-$repo/target/release/php2}
work=$repo/.tmp/wordpress-gate
version=6.8.3
mkdir -p "$work"
cd "$work"

if [ ! -d wordpress ]; then
    curl -sSfL -o wp.tar.gz "https://wordpress.org/wordpress-$version.tar.gz"
    curl -sSfL -o wp.tar.gz.sha1 "https://wordpress.org/wordpress-$version.tar.gz.sha1"
    [ "$(shasum -a 1 wp.tar.gz | cut -d' ' -f1)" = "$(cat wp.tar.gz.sha1)" ] || { echo "wordpress-$version.tar.gz: sha1 mismatch"; exit 1; }
    tar xzf wp.tar.gz
    sed -e "s/'database_name_here'/getenv('WPDBNAME')/" -e "s/'username_here'/'wp'/" -e "s/'password_here'/'wp'/" \
        -e "s/'localhost'/getenv('WPDBHOST')/" wordpress/wp-config-sample.php > wordpress/wp-config.php
fi
cp "$here/page.php" "$here/install.php" .

docker network inspect p8net >/dev/null 2>&1 || docker network create p8net >/dev/null
if ! docker ps --format '{{.Names}}' | grep -qx p8-mariadb; then
    docker rm -f p8-mariadb >/dev/null 2>&1 || true
    docker run -d --name p8-mariadb --network p8net -p 127.0.0.1:3307:3306 \
        -e MARIADB_ROOT_PASSWORD=p8root -e MARIADB_USER=wp -e MARIADB_PASSWORD=wp mariadb:11.4 >/dev/null
    until docker exec p8-mariadb mariadb -uroot -pp8root -e 'SELECT 1' >/dev/null 2>&1; do sleep 1; done
fi
if ! docker image inspect p8-php-mysqli >/dev/null 2>&1; then
    printf 'FROM php:8.4-cli\nRUN docker-php-ext-install mysqli\n' | docker build -q -t p8-php-mysqli - >/dev/null
fi

sql() { docker exec p8-mariadb mariadb -uroot -pp8root "$@"; }
for db in wp_php wp_p8; do
    sql -e "DROP DATABASE IF EXISTS $db; CREATE DATABASE $db; GRANT ALL ON $db.* TO 'wp'@'%';"
done
# PHP runs at the same path as p8 and as the same user: WordPress prints
# its paths, and picks how it would write files by who owns them. It also
# lists directories in the order the system gives (a theme's parts, say):
# a tmpfs lists in reverse order of creation, so the files go in reversed
# from this system's order.
top=/$(printf '%s' "$work" | cut -d/ -f2)
php_ref() { { printf '%s\n' "$1"; find wordpress | tail -r; } | COPYFILE_DISABLE=1 tar -cf - --no-recursion -T - 2>/dev/null | docker run --rm -i --network p8net --user "$(id -u):$(id -g)" \
    --tmpfs "$top:uid=$(id -u),gid=$(id -g),exec" -e WPURL="${WPURL:-/}" -e WPUSER="${WPUSER:-}" -e WPDBHOST=p8-mariadb -e WPDBNAME=wp_php p8-php-mysqli \
    bash -c "mkdir -p '$work' && cd '$work' && tar xf - 2>/dev/null && php $1"; }
p8() { WPURL="${WPURL:-/}" WPUSER="${WPUSER:-}" WPDBHOST=127.0.0.1:3307 WPDBNAME=wp_p8 "$php2" "$1"; }
# The time of the request, which wp-admin prints: as timestamps (its user
# settings, the editor's post lock), as the clock, and as the date of the
# auto-draft it makes (the test content is dated 2026-01-01, so today's
# date is only ever the clock).
today=$(date -u +%Y-%m-%d)
clock() { sed -E -e 's/"time":"[0-9]+"/"time":"T"/g' -e 's/"activePostLock":"[0-9]+:/"activePostLock":"T:/g' -e 's#(Universal|Local) time is <code>[^<]*</code>#\1 time is <code>T</code>#g' \
    -e "s/${today}([T ])[0-9]{2}:[0-9]{2}:[0-9]{2}/${today}\1T/g" "$1"; }

php_ref install.php > install.php.out
p8 install.php > install.p8.out
diff install.p8.out install.php.out || { echo "FAIL: wp_install() differs"; exit 1; }
cat install.p8.out

# The installs differ only in their dates and their generated salts (which
# nonces are made from): make those the same so the pages compare.
for db in wp_php wp_p8; do
    sql "$db" -e "UPDATE wp_posts SET post_date='2026-01-01 12:00:00', post_date_gmt='2026-01-01 12:00:00', post_modified='2026-01-01 12:00:00', post_modified_gmt='2026-01-01 12:00:00'; UPDATE wp_comments SET comment_date='2026-01-01 12:00:00', comment_date_gmt='2026-01-01 12:00:00';"
done
sql -e "UPDATE wp_p8.wp_options o JOIN wp_php.wp_options r USING (option_name) SET o.option_value = r.option_value WHERE option_name LIKE '%\\_key' OR option_name LIKE '%\\_salt';"

# Requests as "user:url": user 0 is signed out, 1 is the admin. They run in
# this order under both, so rows they add (wp-admin's auto-drafts) match.
failed=0
for req in 0:/ 0:/?p=1 0:/?feed=rss2 0:/?rest_route=/ 0:/?s=hello 0:/wp-login.php \
    1:/wp-admin/ 1:/wp-admin/edit.php 1:/wp-admin/post-new.php 1:/wp-admin/options-general.php 1:/wp-admin/users.php 1:/wp-admin/plugins.php; do
    user=${req%%:*} url=${req#*:}
    [ "$user" = 0 ] && user=
    name=$(printf '%s' "$url" | tr -c 'a-zA-Z0-9' '_')
    WPUSER=$user WPURL=$url php_ref page.php | clock /dev/stdin > "page$name.php.html"
    { WPUSER=$user WPURL=$url p8 page.php || true; } | clock /dev/stdin > "page$name.p8.html"
    if cmp -s "page$name.p8.html" "page$name.php.html"; then
        echo "PASS $url ($(wc -c < "page$name.p8.html" | tr -d ' ') bytes, sha256 $(shasum -a 256 "page$name.p8.html" | cut -c1-16))"
    else
        echo "FAIL $url (p8 $(wc -c < "page$name.p8.html" | tr -d ' ') bytes, PHP $(wc -c < "page$name.php.html" | tr -d ' '))"
        diff <(sed 's/></>\n</g' "page$name.p8.html") <(sed 's/></>\n</g' "page$name.php.html") | head -20 || true
        failed=1
    fi
done
exit $failed
