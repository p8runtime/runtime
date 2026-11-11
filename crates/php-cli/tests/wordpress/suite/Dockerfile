# The reference PHP (BASE: crates/php-test/php-src/reference.sh) as WordPress's test suite wants it (gd with jpeg/png/webp/avif/freetype, mysqli, exif, intl, zip).
ARG BASE
FROM ${BASE}
RUN apt-get update && apt-get install -y --no-install-recommends libpng-dev libjpeg62-turbo-dev libfreetype6-dev libwebp-dev libavif-dev libicu-dev libzip-dev \
 && docker-php-ext-configure gd --with-jpeg --with-freetype --with-webp --with-avif \
 && docker-php-ext-install -j4 gd mysqli exif intl zip \
 && rm -rf /var/lib/apt/lists/*
