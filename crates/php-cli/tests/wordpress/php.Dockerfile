# The reference PHP (BASE: tests/php-src/reference.sh, the release
# p8-test-suite pins) for the gate: mysqli, and gd with the formats p8's gd
# has (png, jpeg, gif, webp; no avif or freetype), so what WordPress offers
# for images (wp_image_editor_supports) is the same under both. intl too,
# as p8 has it: WooCommerce sorts its country list with Collator when
# intl is there.
ARG BASE
FROM ${BASE}
RUN apt-get update && apt-get install -y --no-install-recommends libpng-dev libjpeg62-turbo-dev libwebp-dev libicu-dev \
 && docker-php-ext-configure gd --with-jpeg --with-webp \
 && docker-php-ext-install -j4 gd mysqli intl \
 && rm -rf /var/lib/apt/lists/*
