# The reference PHP (BASE: tests/php-src/reference.sh, the release
# p8-test-suite pins) for the apps' own test suites
# (crates/php-cli/tests/apps/run.sh): that image plus the extensions those apps require (intl, gd, zip,
# bcmath, gmp, the database drivers, sockets, pcntl, exif, calendar), and
# no memory limit, as the apps' CI runs their tests (their bootstraps start
# PHP subprocesses that -d memory_limit does not reach).
ARG BASE
FROM ${BASE}
RUN apt-get update && apt-get install -y --no-install-recommends \
        libicu-dev libpng-dev libjpeg62-turbo-dev libwebp-dev libfreetype6-dev libzip-dev libgmp-dev libpq-dev git unzip \
 && docker-php-ext-configure gd --with-jpeg --with-webp --with-freetype \
 && docker-php-ext-install -j4 intl gd zip bcmath gmp pdo_mysql pdo_pgsql mysqli sockets pcntl exif calendar \
 && echo 'memory_limit = -1' > /usr/local/etc/php/conf.d/memory.ini \
 && rm -rf /var/lib/apt/lists/*
