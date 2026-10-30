# PHP 8.4 as MediaWiki 1.46 requires it (composer.json "ext-*"): the stock
# image plus intl and calendar.
FROM php:8.4-cli
RUN apt-get update && apt-get install -y --no-install-recommends libicu-dev \
 && docker-php-ext-install -j4 intl calendar \
 && rm -rf /var/lib/apt/lists/*
