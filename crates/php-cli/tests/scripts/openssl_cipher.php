<?php
// openssl_encrypt/openssl_decrypt over every mode, with PHP's IV and key fixing.
var_dump(extension_loaded('openssl'), OPENSSL_VERSION_NUMBER >= 0x30000000, is_string(OPENSSL_VERSION_TEXT));
$iv16 = "0123456789abcdef";
foreach (['aes-128-cbc', 'aes-192-cbc', 'aes-256-cbc', 'aes-128-ecb', 'aes-256-ctr', 'aes-128-cfb', 'aes-128-cfb8', 'aes-128-cfb1', 'aes-128-ofb', 'camellia-256-cbc', 'chacha20', 'AES-256-CBC', 'aes256'] as $m) {
    $iv = substr(str_repeat($iv16, 2), 0, openssl_cipher_iv_length($m));
    $ct = openssl_encrypt("The quick brown fox jumps over the lazy dog", $m, "secret key", 0, $iv);
    echo $m, " ", openssl_cipher_key_length($m), " ", openssl_cipher_iv_length($m), " ", $ct, "\n";
    var_dump(openssl_decrypt($ct, $m, "secret key", 0, $iv));
}
foreach (['des-ede3-cbc', 'des-ede-cbc', 'des-ede3-cfb', 'des-ede3-ofb'] as $m) {
    $ct = openssl_encrypt("abcdefghijklmnopqrstuvwxyz", $m, "k", 0, "01234567");
    echo $m, " ", $ct, "\n";
    var_dump(openssl_decrypt($ct, $m, "k", 0, "01234567"));
}
// AEAD modes: tags of several lengths, AAD, wrong tags
$ct = openssl_encrypt("abcdefghijklmnopqrstuvwxyz", "chacha20-poly1305", "k", 0, "0123456789ab", $t, "aad", 10);
var_dump($ct, bin2hex($t), openssl_decrypt($ct, "chacha20-poly1305", "k", 0, "0123456789ab", $t, "aad"));
var_dump(openssl_decrypt($ct, "chacha20-poly1305", "k", 0, "0123456789ab", $t, "other"));
var_dump(openssl_encrypt("abcdefghijklmnopqrstuvwxyz", "chacha20-poly1305", "k", 0, "01234", $t, "aad"));
$ct = openssl_encrypt("abcdefghijklmnopqrstuvwxyz", "aes-192-gcm", "k", 0, "01234", $t, "aad", 5);
var_dump($ct, bin2hex($t), openssl_decrypt($ct, "aes-192-gcm", "k", 0, "01234", $t, "aad"));
$ct = openssl_encrypt("abcdefghijklmnopqrstuvwxyz", "aes-128-ccm", "k", 0, "0123456", $t, "aad", 4);
var_dump($ct, bin2hex($t), openssl_decrypt($ct, "aes-128-ccm", "k", 0, "0123456", $t, "aad"));
var_dump(openssl_decrypt($ct, "aes-128-ccm", "k", 0, "0123456", "xxxx", "aad"));
var_dump(openssl_encrypt("x", "aes-128-gcm", "k", 0, "0123456789ab"));
var_dump(openssl_encrypt("x", "aes-128-gcm", "k", 0, "0123456789ab", $t, "", 17));
// IV and key fixing, padding, errors and the error queue
var_dump(openssl_encrypt("abc", "aes-128-ecb", "k", 0, "xx"));
var_dump(openssl_encrypt("abc", "AES-128-ECB", "k"));
var_dump(openssl_encrypt("abc", "aes-128-cbc", "k"));
var_dump(openssl_encrypt("abc", "aes-128-cbc", "k", 0, "short"));
var_dump(openssl_encrypt("abc", "aes-128-cbc", "k", 0, "waytoolongivwaytoolongiv"));
var_dump(openssl_decrypt("abc", "aes-128-cbc", "k", 0, "", "tag"));
var_dump(openssl_encrypt("abc", "aes-128-cbc", "k", OPENSSL_DONT_ZERO_PAD_KEY, $iv16));
while ($e = openssl_error_string()) echo $e, "\n";
var_dump(bin2hex(openssl_encrypt("abc", "aes-128-cbc", str_repeat("k", 40), OPENSSL_RAW_DATA, $iv16)));
while ($e = openssl_error_string()) echo $e, "\n";
var_dump(openssl_encrypt("abc", "aes-128-cbc", "k", OPENSSL_RAW_DATA | OPENSSL_ZERO_PADDING, $iv16));
var_dump(openssl_encrypt("0123456789abcdef", "aes-128-cbc", "k", OPENSSL_ZERO_PADDING, $iv16));
var_dump(openssl_decrypt("abcdefghabcdefghabcdefghabcdefgh", "aes-128-cbc", "k", OPENSSL_RAW_DATA, $iv16));
var_dump(openssl_decrypt("abc", "aes-128-cbc", "k", OPENSSL_RAW_DATA, $iv16));
while ($e = openssl_error_string()) echo $e, "\n";
var_dump(openssl_error_string());
var_dump(openssl_encrypt("abc", "nope", "k"));
var_dump(openssl_cipher_iv_length("nope"));
try {
    openssl_cipher_iv_length("");
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
try {
    openssl_decrypt("abc", "", "k");
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
// digests, PBKDF2, the method lists, random bytes
foreach (['md5', 'sha1', 'SHA256', 'sha384', 'sha512', 'sha512-256', 'sha3-256', 'ripemd160', 'whirlpool', 'blake2b512', 'sm3', 'md4', 'shake128', 'RSA-SHA1'] as $md) {
    echo $md, " ", openssl_digest("abc", $md), "\n";
}
var_dump(bin2hex(openssl_digest("abc", "sha1", true)));
var_dump(openssl_digest("abc", "nope"));
var_dump(bin2hex(openssl_pbkdf2("password", "salt", 20, 2)), bin2hex(openssl_pbkdf2("password", "salt", 32, 3, "sha256")));
var_dump(in_array('sha384', openssl_get_md_methods()), in_array('RSA-SHA256', openssl_get_md_methods(true)), in_array('RSA-SHA256', openssl_get_md_methods()));
var_dump(in_array('aes-256-gcm', openssl_get_cipher_methods()), in_array('aes256', openssl_get_cipher_methods(true)), in_array('prime256v1', openssl_get_curve_names()));
var_dump(strlen(openssl_random_pseudo_bytes(17, $strong)), $strong);
try {
    openssl_random_pseudo_bytes(0);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
