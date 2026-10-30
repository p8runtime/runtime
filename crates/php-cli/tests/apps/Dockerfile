# PHP 8.4 for the apps' own test suites (crates/php-cli/tests/apps/run.sh):
# the stock image plus the extensions those apps require (intl, gd, zip,
# bcmath, gmp, the database drivers, sockets, pcntl, exif, calendar).
FROM php:8.4-cli
RUN apt-get update && apt-get install -y --no-install-recommends \
        libicu-dev libpng-dev libjpeg62-turbo-dev libwebp-dev libfreetype6-dev libzip-dev libgmp-dev git unzip \
 && docker-php-ext-configure gd --with-jpeg --with-webp --with-freetype \
 && docker-php-ext-install -j4 intl gd zip bcmath gmp pdo_mysql mysqli sockets pcntl exif calendar \
 && rm -rf /var/lib/apt/lists/*
