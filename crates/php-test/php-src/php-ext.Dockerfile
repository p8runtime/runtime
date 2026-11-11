# The reference PHP (php-src.Dockerfile's image, BASE) made extensible the
# way the official php:<version>-cli image is: docker-library's
# docker-php-ext-configure / -install / -enable and docker-php-source, and
# the release's own sources as /usr/src/php.tar.xz for them to build from.
# The GitHub archive is a git tree, so its lexer and parser files are
# generated first (scripts/dev/genfiles, as makedist does for a release
# tarball): extensions such as pdo_mysql ship them pre-built.
# The app harnesses (crates/php-cli/tests) start from this image and add
# the extensions each app needs with docker-php-ext-install. Built by
# reference.sh.
ARG BASE
# docker-library's helper scripts are version-independent shell; they are
# taken from their 8.6 image.
FROM php:8.6-rc-cli AS helpers
FROM ${BASE}
COPY --from=helpers /usr/local/bin/docker-php-ext-configure /usr/local/bin/docker-php-ext-enable /usr/local/bin/docker-php-ext-install /usr/local/bin/docker-php-source /usr/local/bin/
ARG PHP_TAG
ARG PHP_SHA256
ENV PHPIZE_DEPS="autoconf dpkg-dev file g++ gcc libc-dev make pkg-config re2c"
RUN set -eux; test -n "$PHP_TAG"; test -n "$PHP_SHA256"; cd /usr/src; \
  curl -fsSL -o php.tar.gz "https://github.com/php/php-src/archive/refs/tags/$PHP_TAG.tar.gz"; \
  echo "$PHP_SHA256 *php.tar.gz" | sha256sum -c -; \
  mkdir php; tar -xzf php.tar.gz -C php --strip-components=1; rm php.tar.gz; \
  (cd php && ./scripts/dev/genfiles > /dev/null); \
  tar -cJf php.tar.xz -C php .; rm -rf php; \
  docker-php-source extract; test -f /usr/src/php/ext/pdo_mysql/mysql_sql_parser.c; docker-php-source delete
