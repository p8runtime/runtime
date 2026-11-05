# The real PHP that p8 is compared with, shared by make-target.sh (php-src's
# tests) and the app harnesses (crates/php-cli/tests: the WordPress gate, the
# WordPress and MediaWiki suites, apps/run.sh). Sourced, not run.
#
# The version is the one p8-test-suite pins (php-src.json: the php-src tag
# and the SHA-256 of its GitHub archive), so moving the target moves every
# comparison with it.
#
#   php_reference <p8-test-suite checkout>
#     builds p8-target/php:<tag> (php-src.Dockerfile, as docker-library builds
#     php:<version>-cli) and p8-target/php-ext:<tag> (php-ext.Dockerfile: the
#     same image plus docker-library's docker-php-ext-* helpers and the
#     release's sources, so harness images add extensions the usual way).
#     Sets php_tag, php_sha256, php_ref (the first image), php_ref_ext (the
#     second) and php_version (its PHP_VERSION).
#
#   php_image <name> <Dockerfile> <context>
#     after php_reference: builds <Dockerfile> (which starts `ARG BASE` /
#     `FROM ${BASE}`) on php_ref_ext and prints its tag,
#     <name>:<12 hex digits of the Dockerfile and the base image>; an image
#     already built for both is reused.
#
# Needs docker and python3.
php_reference() {
  local suite=$1 here
  here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
  [ -f "$suite/php-src.json" ] || { echo "$suite/php-src.json: no such file (pass the p8-test-suite checkout)" >&2; return 1; }
  read -r php_tag php_sha256 < <(python3 -I -c 'import json,sys; p=json.load(open(sys.argv[1])); print(p["tag"], p["sha256"])' "$suite/php-src.json")
  php_ref="p8-target/php:$php_tag"
  php_ref_ext="p8-target/php-ext:$php_tag"
  # Built every time: docker's layer cache makes an unchanged build instant,
  # and a changed Dockerfile is never answered by an old image.
  docker build -q -t "$php_ref" --build-arg PHP_TAG="$php_tag" --build-arg PHP_SHA256="$php_sha256" - < "$here/php-src.Dockerfile" > /dev/null
  docker build -q -t "$php_ref_ext" --build-arg BASE="$php_ref" --build-arg PHP_TAG="$php_tag" --build-arg PHP_SHA256="$php_sha256" - < "$here/php-ext.Dockerfile" > /dev/null
  php_version=$(docker run --rm "$php_ref_ext" php -r 'echo PHP_VERSION;')
}

php_image() {
  local name=$1 dockerfile=$2 context=$3 base key image
  base=$(docker image inspect -f '{{.Id}}' "$php_ref_ext")
  key=$( { cat "$dockerfile"; echo "$base"; } | shasum -a 256 | cut -c1-12)
  image="$name:$key"
  # (Called in $(...), where set -e does not reach: fail explicitly.)
  docker image inspect "$image" > /dev/null 2>&1 \
    || docker build -q -t "$image" --build-arg BASE="$php_ref_ext" -f "$dockerfile" "$context" > /dev/null \
    || { echo "$dockerfile: docker build failed" >&2; return 1; }
  echo "$image"
}
