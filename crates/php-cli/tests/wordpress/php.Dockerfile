# PHP 8.4 for the gate: mysqli, and gd with the formats p8's gd has
# (png, jpeg, gif, webp; no avif or freetype), so what WordPress offers
# for images (wp_image_editor_supports) is the same under both.
FROM php:8.4-cli
RUN apt-get update && apt-get install -y --no-install-recommends libpng-dev libjpeg62-turbo-dev libwebp-dev \
 && docker-php-ext-configure gd --with-jpeg --with-webp \
 && docker-php-ext-install -j4 gd mysqli \
 && rm -rf /var/lib/apt/lists/*
