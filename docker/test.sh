#!/usr/bin/env bash
# The Docker flow, end to end: build the p8 image from this checkout, build
# docker/example on it (`p8 compile /app` in the image), run it, and check
# that it serves its pages and its static file from the compiled package;
# that an edited file runs its new code; then the same for WordPress
# (docker/wordpress) with MariaDB in a second container, installed through
# its own installer page. Exit 0 when every check passes.
#
# Usage: docker/test.sh [-j <cargo jobs>] [--no-wordpress]
# Needs docker (with BuildKit) and curl. Everything it starts or builds is
# named p8-test-<pid>-* and removed at the end, pass or fail; the images it
# pulls (rust, distroless, alpine, mariadb) stay in Docker's cache.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(dirname "$here")
jobs=
wordpress=1
while [ $# -gt 0 ]; do
    case $1 in
        -j) jobs=$2; shift 2 ;;
        --no-wordpress) wordpress=; shift ;;
        *) echo "usage: docker/test.sh [-j <cargo jobs>] [--no-wordpress]" >&2; exit 64 ;;
    esac
done

id=p8-test-$$
work=$repo/.tmp/docker-test-$$
containers=()
images=()
network=
cleanup() {
    status=$?
    for c in ${containers[@]+"${containers[@]}"}; do docker rm -f "$c" >/dev/null 2>&1 || true; done
    [ -z "$network" ] || docker network rm "$network" >/dev/null 2>&1 || true
    for i in ${images[@]+"${images[@]}"}; do docker rmi "$i" >/dev/null 2>&1 || true; done
    [ ! -d "$work" ] || find "$work" -delete
    if [ "$status" -eq 0 ]; then echo "PASS"; else echo "FAIL (exit $status)"; fi
    exit "$status"
}
trap cleanup EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
mkdir -p "$work"

# get <url> [curl options...]: the body into $work/body, the status into $code.
get() {
    local url=$1
    shift
    code=$(curl -sS -o "$work/body" -D "$work/headers" -w '%{http_code}' "$@" "$url")
}
# expect_status <code> <what>
expect_status() { [ "$code" = "$1" ] || { cat "$work/body" >&2; fail "$2: status $code, expected $1"; }; }
# wait_up <container> <url>: until the server answers (30 s at most).
wait_up() {
    for _ in $(seq 150); do
        curl -s -o /dev/null "$2" && return 0
        docker inspect -f '{{.State.Running}}' "$1" | grep -qx true || { docker logs "$1" >&2; fail "$1 exited"; }
        sleep 0.2
    done
    docker logs "$1" >&2
    fail "$1 does not answer on $2"
}
# log_lines <container> <pattern>: the request log lines that match.
log_lines() { docker logs "$1" 2>&1 | grep -F -- "$2" || true; }
port_of() { docker port "$1" 8000/tcp | head -n1 | sed 's/.*://'; }
random() { od -An -N16 -tx1 /dev/urandom | tr -d ' \n'; }

# 1. The p8 image.
echo "== build the p8 image (docker/Dockerfile)"
images+=("$id-p8")
SECONDS=0
docker build -q ${jobs:+--build-arg JOBS=$jobs} -f "$here/Dockerfile" -t "$id-p8" "$repo" >/dev/null
echo "built in ${SECONDS} s; image size $(docker image inspect -f '{{.Size}}' "$id-p8") bytes"
docker run --rm "$id-p8"
docker run --rm --entrypoint id "$id-p8" 2>/dev/null && fail "the image has an id command (expected: p8 only)"
[ "$(docker image inspect -f '{{.Config.User}}' "$id-p8")" = nonroot ] || fail "the image does not run as nonroot"

# 2. The example: `p8 compile /app` in the image, then `p8 serve`.
echo "== build docker/example on it"
images+=("$id-example")
SECONDS=0
docker build -q --build-context p8=docker-image://"$id-p8" -t "$id-example" "$here/example" >/dev/null
echo "built in ${SECONDS} s; image size $(docker image inspect -f '{{.Size}}' "$id-example") bytes"
c=$id-example
containers+=("$c")
docker run -d --name "$c" -p 127.0.0.1::8000 "$id-example" >/dev/null
url=http://127.0.0.1:$(port_of "$c")
wait_up "$c" "$url/style.css"

get "$url/"
expect_status 200 "GET /"
printf 'Page: home\nHello from p8 (PHP 8.6)\n' | cmp -s - "$work/body" || { cat "$work/body" >&2; fail "GET /: wrong body"; }
echo "GET / -> $code: $(tr '\n' '|' < "$work/body")"

get "$url/index.php?page=docs"
expect_status 200 "GET /index.php?page=docs"
printf 'Page: docs\nHello from p8 (PHP 8.6)\n' | cmp -s - "$work/body" || { cat "$work/body" >&2; fail "GET /index.php?page=docs: wrong body"; }
echo "GET /index.php?page=docs -> $code: $(tr '\n' '|' < "$work/body")"

get "$url/style.css"
expect_status 200 "GET /style.css"
cmp -s "$here/example/style.css" "$work/body" || fail "GET /style.css: not the file"
grep -qi '^content-type: text/css' "$work/headers" || { cat "$work/headers" >&2; fail "GET /style.css: not text/css"; }
echo "GET /style.css -> $code, $(wc -c < "$work/body" | tr -d ' ') bytes, text/css"

echo "-- the server's log"
docker logs "$c" 2>&1
[ -z "$(log_lines "$c" 'so every file is compiled from source')" ] || fail "p8 serve did not use the package"
[ "$(log_lines "$c" 'GET / - 0 files compiled from source' | wc -l)" -eq 1 ] || fail "GET / was not served from the package"
[ "$(log_lines "$c" 'GET /index.php?page=docs - 0 files compiled from source' | wc -l)" -eq 1 ] || fail "GET /index.php was not served from the package"

# An edited file runs its new code at the next request, compiled from source.
echo "== edit lib.php in the running container"
sed 's/Hello from p8/Edited in place/' "$here/example/lib.php" > "$work/lib.php"
docker cp "$work/lib.php" "$c:/app/lib.php"
get "$url/"
expect_status 200 "GET / after the edit"
printf 'Page: home\nEdited in place (PHP 8.6)\n' | cmp -s - "$work/body" || { cat "$work/body" >&2; fail "GET / after the edit: old code"; }
echo "GET / -> $code: $(tr '\n' '|' < "$work/body")"
log_lines "$c" 'GET / - 1 file compiled from source' | grep -q . || { docker logs "$c" >&2; fail "the edited file was not compiled from source"; }
log_lines "$c" 'GET / - 1 file compiled from source'

SECONDS=0
docker stop "$c" >/dev/null
echo "docker stop took ${SECONDS} s; exit code $(docker inspect -f '{{.State.ExitCode}}' "$c")"
# Docker sends SIGTERM, then SIGKILL after 10 s: a server that ignores SIGTERM
# (as PID 1 without tini) takes the full 10.
[ "$SECONDS" -lt 5 ] || fail "p8 serve did not stop on SIGTERM"

[ -n "$wordpress" ] || exit 0

# 3. WordPress, with MariaDB in its own container on a network of its own.
echo "== WordPress (docker/wordpress) with MariaDB"
images+=("$id-wordpress")
SECONDS=0
docker build -q --build-context p8=docker-image://"$id-p8" -t "$id-wordpress" "$here/wordpress" >/dev/null
echo "built in ${SECONDS} s; image size $(docker image inspect -f '{{.Size}}' "$id-wordpress") bytes"
network=$id-net
docker network create "$network" >/dev/null
db_password=$(random)
db=$id-db
containers+=("$db")
docker run -d --name "$db" --network "$network" --network-alias db \
    -e MARIADB_DATABASE=wordpress -e MARIADB_USER=wordpress -e MARIADB_PASSWORD="$db_password" -e MARIADB_RANDOM_ROOT_PASSWORD=1 \
    mariadb:11.4@sha256:d1891ede5a559f1086f55053bb8cc1569b0298338c4d72d71725a7f74f6ba620 >/dev/null
for _ in $(seq 120); do
    docker exec "$db" healthcheck.sh --connect --innodb_initialized >/dev/null 2>&1 && break
    sleep 0.5
done
docker exec "$db" healthcheck.sh --connect --innodb_initialized >/dev/null || { docker logs "$db" >&2; fail "MariaDB did not start"; }

w=$id-wordpress
containers+=("$w")
docker run -d --name "$w" --network "$network" -e WORDPRESS_DB_PASSWORD="$db_password" -p 127.0.0.1::8000 "$id-wordpress" >/dev/null
url=http://127.0.0.1:$(port_of "$w")
wait_up "$w" "$url/wp-includes/css/dashicons.min.css"

get "$url/"
expect_status 302 "GET / before the install"
grep -qi '^location: .*/wp-admin/install.php' "$work/headers" || { cat "$work/headers" >&2; fail "GET / before the install: not sent to the installer"; }
echo "GET / -> $code, to $(grep -i '^location:' "$work/headers" | tr -d '\r' | sed 's/^[^:]*: //')"

admin_password=$(random)
get "$url/wp-admin/install.php?step=2" \
    --data-urlencode "weblog_title=p8 blog" --data-urlencode "user_name=admin" \
    --data-urlencode "admin_password=$admin_password" --data-urlencode "admin_password2=$admin_password" \
    --data-urlencode "pw_weak=1" --data-urlencode "admin_email=admin@example.com" --data-urlencode "blog_public=0"
expect_status 200 "POST /wp-admin/install.php?step=2"
grep -q 'Success!' "$work/body" || { cat "$work/body" >&2; fail "the install did not succeed"; }
echo "POST /wp-admin/install.php?step=2 -> $code: Success!"

get "$url/"
expect_status 200 "GET / after the install"
grep -q '<title>p8 blog</title>' "$work/body" || { head -c 2000 "$work/body" >&2; fail "GET /: not the blog's front page"; }
grep -q 'Hello world!' "$work/body" || fail "GET /: the first post is missing"
echo "GET / -> $code, $(wc -c < "$work/body" | tr -d ' ') bytes, <title>p8 blog</title>, 'Hello world!'"

get "$url/?p=1"
expect_status 200 "GET /?p=1"
grep -q 'Hello world!' "$work/body" || fail "GET /?p=1: not the first post"
echo "GET /?p=1 -> $code, $(wc -c < "$work/body" | tr -d ' ') bytes"

echo "-- the server's log"
docker logs "$w" 2>&1
[ -z "$(log_lines "$w" 'so every file is compiled from source')" ] || fail "p8 serve did not use WordPress's package"
log_lines "$w" ': GET / - ' | tail -n1 | grep -qF 'GET / - 0 files compiled from source' || fail "GET / was not served from the package"
[ "$(log_lines "$w" 'GET /?p=1 - 0 files compiled from source' | wc -l)" -eq 1 ] || fail "GET /?p=1 was not served from the package"
