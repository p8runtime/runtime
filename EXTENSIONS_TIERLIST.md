# Extension Tier List + Ergonomics Plan

This document summarizes the extension priorities derived from framework requirements and outlines the runtime ergonomics we need to make them usable in php-rs.

Sources:
- `EXTENSIONS_FRAMEWORKS.md` (Laravel, Symfony, Magento, Drupal, WordPress)

## Tier 0 (must-have)

These are required for common frameworks to boot and for basic compatibility.

- `json`
- `mbstring`
- `ctype`
- `tokenizer`
- `filter`
- `openssl`
- `hash`
- `session`

## Tier 1 (common frameworks + CMS)

- `pdo_mysql`
- `curl`
- `dom`
- `simplexml`
- `xml`
- `gd`
- `iconv`
- `intl`
- `zip`
- `sodium`

## Tier 2 (enterprise / heavier apps)

- `bcmath`
- `soap`
- `xsl`
- `ftp`

## Strategy: native core + PHP stdlib

We will expose a small native host surface and build PHP-level ergonomics on top.

### 1) Native ops: `gop_*` (sync)

- `gop_*` are Rust-native, synchronous, PHP-friendly operations.
- The API is intentionally narrow and stable.
- These ops power both core extensions and PHP stdlib wrappers.

### 2) PHP stdlib facade

- Provide a built-in PHP stdlib with clean namespaces.
- Example pattern:

```
use function mysql\{ connect, query, close };
```

- This stdlib will map to `gop_*` operations.
- Namespaces used by stdlib are reserved (ex: `mysql`, `fs`, `net`, `crypto`, `http`).

### 3) Error model

- `gop_*` should return clear error types that map to PHP exceptions or warnings.
- Errors must include machine-readable codes for interoperability with tests.

## Extension Ergonomics (Initial Plan)

### json
- `json_encode`, `json_decode`, `json_last_error`, `json_last_error_msg`

### mbstring
- `mb_strlen`, `mb_substr`, `mb_strtolower`, `mb_strtoupper`, `mb_internal_encoding`

### ctype
- `ctype_alpha`, `ctype_alnum`, `ctype_digit`, `ctype_space`, `ctype_upper`, `ctype_lower`

### tokenizer
- `token_get_all` (sufficient to parse framework templates and annotations)

### filter
- `filter_var`, `filter_input` (basic validation + sanitization)

### openssl
- `openssl_random_pseudo_bytes`
- `openssl_encrypt`, `openssl_decrypt` (AES)

### hash
- `hash`, `hash_hmac`, `hash_equals`

### session
- `session_start`, `session_id`, `session_get_cookie_params`, `session_destroy`

### pdo_mysql
- `PDO::__construct`, `PDO::prepare`, `PDO::query`, `PDOStatement::execute`, `PDOStatement::fetch`

### curl
- `curl_init`, `curl_setopt`, `curl_exec`, `curl_getinfo`, `curl_error`, `curl_close`

### dom / xml / simplexml
- `DOMDocument`, `DOMElement`, `simplexml_load_string`, `simplexml_load_file`

### gd
- `imagecreatefrompng`, `imagecreatefromjpeg`, `imagepng`, `imagejpeg`, `imagescale`

### iconv / intl
- `iconv`, `intl` basics for locale + formatting (expand as needed)

### zip
- `ZipArchive` (open, extract, add)

### sodium
- `sodium_crypto_secretbox`, `sodium_crypto_secretbox_open`, `sodium_randombytes_buf`

### bcmath / soap / xsl / ftp
- Implement after Tier 0/1 are solid.

## Implementation Notes

- Prefer direct Rust implementations where possible.
- Keep `gop_*` APIs sync and minimal.

## Next Steps

1) Map current php-rs coverage against Tier 0/1.
2) Add failing tests for missing extension APIs.
3) Implement `gop_*` primitives for the highest-impact extension gaps.
