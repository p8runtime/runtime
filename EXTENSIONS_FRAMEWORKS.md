# PHP Extension Requirements (Framework Scan)

Generated: 2026-01-04T06:20:02Z

Sources: composer.json require ext-* keys (framework repos).

## Per-framework requirements

### Laravel

Source: https://raw.githubusercontent.com/laravel/framework/11.x/composer.json
PHP: ^8.2
Extensions:
ctype, filter, hash, mbstring, openssl, session, tokenizer

### Symfony

Source: https://raw.githubusercontent.com/symfony/symfony/7.2/composer.json
PHP: >=8.2
Extensions:
xml

### Magento

Source: https://raw.githubusercontent.com/magento/magento2/2.4-develop/composer.json
PHP: ~8.2.0||~8.3.0||~8.4.0
Extensions:
bcmath, ctype, curl, dom, ftp, gd, hash, iconv, intl, mbstring, openssl, pdo_mysql, simplexml, soap, sodium, xsl, zip

### Drupal

Source: https://raw.githubusercontent.com/drupal/drupal/10.3.x/composer.json
Extensions:
(none found in composer.json)

### WordPress

Source: https://raw.githubusercontent.com/WordPress/wordpress-develop/trunk/composer.json
PHP: >=7.2.24
Extensions:
hash, json

## Aggregate (union)

bcmath, ctype, curl, dom, filter, ftp, gd, hash, iconv, intl, json, mbstring, openssl, pdo_mysql, session, simplexml, soap, sodium, tokenizer, xml, xsl, zip

## Frequency

| Extension | Count |
| --- | --- |
| bcmath | 1 |
| ctype | 2 |
| curl | 1 |
| dom | 1 |
| filter | 1 |
| ftp | 1 |
| gd | 1 |
| hash | 3 |
| iconv | 1 |
| intl | 1 |
| json | 1 |
| mbstring | 2 |
| openssl | 2 |
| pdo_mysql | 1 |
| session | 1 |
| simplexml | 1 |
| soap | 1 |
| sodium | 1 |
| tokenizer | 1 |
| xml | 1 |
| xsl | 1 |
| zip | 1 |
