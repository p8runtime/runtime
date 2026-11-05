# The reference PHP (BASE: tests/php-src/reference.sh) as MediaWiki 1.46
# requires it (composer.json "ext-*"): that image plus intl and calendar.
ARG BASE
FROM ${BASE}
RUN apt-get update && apt-get install -y --no-install-recommends libicu-dev \
 && docker-php-ext-install -j4 intl calendar \
 && rm -rf /var/lib/apt/lists/*
