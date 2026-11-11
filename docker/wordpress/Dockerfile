# WordPress on p8. Build from this folder after the p8 image:
#
#   docker build -t p8-wordpress docker/wordpress
#
# The database is MariaDB in its own container, as in the WordPress gate
# (crates/php-cli/tests/wordpress/gate.sh); docker/README.md has the commands.

# WordPress itself, checked against its sha256 (the release the gate pins).
FROM alpine:3.22@sha256:49340b346798875b5592e5d420241f2f9cfba5f47e62ebb1446a1b34734e898f AS wordpress
ADD --checksum=sha256:881e53a4681ff2cd911651b11cccdede5ae477ddf0765af5d6a1a8b219a1d835 \
    https://wordpress.org/wordpress-7.1.4.tar.gz /wordpress.tar.gz
# wp-config.php from WordPress's sample: the database is `wordpress` on the
# host `db`, as the user `wordpress`; its password is the container's
# WORDPRESS_DB_PASSWORD, chosen when the containers start (never in the image).
RUN tar xzf /wordpress.tar.gz -C / \
    && sed -e "s/'database_name_here'/'wordpress'/" -e "s/'username_here'/'wordpress'/" \
        -e "s/'password_here'/getenv('WORDPRESS_DB_PASSWORD')/" -e "s/'localhost'/'db'/" \
        /wordpress/wp-config-sample.php > /wordpress/wp-config.php

FROM p8
COPY --from=wordpress --chown=nonroot:nonroot /wordpress /app
RUN ["p8", "compile", "/app"]
CMD ["serve", "/app", "--listen", "0.0.0.0:8000"]
