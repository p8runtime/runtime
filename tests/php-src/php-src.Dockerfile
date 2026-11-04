# The real PHP that make-target.sh runs php-src's tests with: the php-src
# release named in p8-test-suite's php-src.json (its GitHub archive, checked
# against its SHA-256), built the way docker-library/php builds the official
# php:<version>-cli image (Debian trixie, same packages, CFLAGS and configure
# flags as their 8.6 Dockerfile).
#
# Two differences from that image:
# - The archive is a git tree, not a release tarball, so `buildconf` runs
#   first (bison + re2c + autoconf). This also works for a release candidate
#   that is tagged before its tarball is published.
# - `--enable-mbstring` (with libonig-dev) is kept, exactly as in
#   docker-library's 8.4 and 8.5 Dockerfiles. Their 8.6 images dropped it
#   together with PEAR (docker-library/php#1654); p8 implements mbstring and
#   its target has always included the mbstring tests.
FROM debian:trixie-slim
ARG PHP_TAG
ARG PHP_SHA256
RUN set -eux; { echo 'Package: php*'; echo 'Pin: release *'; echo 'Pin-Priority: -1'; } > /etc/apt/preferences.d/no-debian-php
RUN set -eux; apt-get update; apt-get install -y --no-install-recommends \
    autoconf bison ca-certificates curl dpkg-dev file g++ gcc libc-dev make pkg-config re2c xz-utils \
    libargon2-dev libcurl4-openssl-dev libonig-dev libreadline-dev libsodium-dev libsqlite3-dev libssl-dev libxml2-dev zlib1g-dev; \
  apt-get dist-clean
ENV PHP_INI_DIR=/usr/local/etc/php
RUN set -eux; mkdir -p "$PHP_INI_DIR/conf.d" /var/www/html; chown www-data:www-data /var/www/html; chmod 1777 /var/www/html
ENV PHP_CFLAGS="-fstack-protector-strong -fpic -fpie -O2 -D_LARGEFILE_SOURCE -D_FILE_OFFSET_BITS=64"
ENV PHP_CPPFLAGS="$PHP_CFLAGS" PHP_LDFLAGS="-Wl,-O1 -pie"
RUN set -eux; test -n "$PHP_TAG"; test -n "$PHP_SHA256"; \
  mkdir -p /usr/src/php; cd /usr/src; \
  curl -fsSL -o php.tar.gz "https://github.com/php/php-src/archive/refs/tags/$PHP_TAG.tar.gz"; \
  echo "$PHP_SHA256 *php.tar.gz" | sha256sum -c -; \
  tar -xzf php.tar.gz -C /usr/src/php --strip-components=1; rm php.tar.gz; \
  cd /usr/src/php; ./buildconf --force; \
  export CFLAGS="$PHP_CFLAGS" CPPFLAGS="$PHP_CPPFLAGS" LDFLAGS="$PHP_LDFLAGS" \
    PHP_BUILD_PROVIDER='https://github.com/docker-library/php' PHP_UNAME='Linux - Docker'; \
  gnuArch="$(dpkg-architecture --query DEB_BUILD_GNU_TYPE)"; debMultiarch="$(dpkg-architecture --query DEB_BUILD_MULTIARCH)"; \
  if [ ! -d /usr/include/curl ]; then ln -sT "/usr/include/$debMultiarch/curl" /usr/local/include/curl; fi; \
  ./configure --build="$gnuArch" --sysconfdir="${PHP_INI_DIR%/php}" \
    --with-config-file-path="$PHP_INI_DIR" --with-config-file-scan-dir="$PHP_INI_DIR/conf.d" \
    --enable-option-checking=fatal --with-mhash --enable-mbstring --enable-mysqlnd \
    --with-password-argon2 --with-sodium=shared --with-pdo-sqlite=/usr --with-sqlite3=/usr \
    --with-curl --with-iconv --with-openssl --with-readline --with-zlib \
    --enable-phpdbg --enable-phpdbg-readline --with-libdir="lib/$debMultiarch" \
    --enable-embed --enable-zts --disable-zend-signals; \
  make -j "$(nproc)"; make install; make clean; cp -v php.ini-* "$PHP_INI_DIR/"; \
  cd /; rm -rf /usr/src/php; \
  echo 'extension=sodium' > "$PHP_INI_DIR/conf.d/docker-php-ext-sodium.ini"; \
  php --version; php -r 'exit(extension_loaded("mbstring") && extension_loaded("sodium") ? 0 : 1);'
CMD ["php", "-a"]
