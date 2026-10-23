<?php
$algos = ['md5', 'sha1', 'sha256', 'sha512', 'crc32b', 'fnv1a64'];
foreach ($algos as $a) {
    if (!in_array($a, hash_algos())) { echo "$a: not available\n"; continue; }
    $ctx = hash_init($a);
    hash_update($ctx, 'The quick brown fox ');
    $copy = hash_copy($ctx);
    hash_update($ctx, 'jumps over the lazy dog');
    echo $a, ' ', hash_final($ctx), ' ', hash_final($copy, false) === hash($a, 'The quick brown fox ') ? 'copy-ok' : 'copy-bad', "\n";
}
$h = hash_init('sha256', HASH_HMAC, 'secret');
hash_update($h, 'data');
echo hash_final($h), "\n", hash_hmac('sha256', 'data', 'secret'), "\n";
var_dump(get_class($h));
try { hash_update($h, 'x'); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { hash_init('crc32b', HASH_HMAC, 'k'); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { hash_init('sha256', HASH_HMAC); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { hash_init('nope'); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
echo bin2hex(hash_hkdf('sha256', 'input key', 32, 'info', 'salt')), "\n";
echo bin2hex(hash_hkdf('sha1', 'ikm')), "\n";
try { hash_hkdf('sha256', ''); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { hash_hkdf('sha256', 'k', -1); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { hash_hkdf('sha256', 'k', 10000); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
echo hash_pbkdf2('sha256', 'password', 'salt', 1000), "\n";
echo hash_pbkdf2('sha1', 'password', 'salt', 2, 20), "\n";
echo bin2hex(hash_pbkdf2('sha1', 'password', 'salt', 2, 10, true)), "\n";
try { hash_pbkdf2('sha1', 'p', 's', 0); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$data = tempnam(sys_get_temp_dir(), 'h8'); file_put_contents($data, str_repeat("abc\n", 1000));
$c = hash_init('md5'); hash_update_file($c, $data); echo hash_final($c), ' ', md5_file($data), "\n";
$fp = fopen($data, 'r'); $c = hash_init('sha1'); var_dump(hash_update_stream($c, $fp, 10)); echo hash_final($c), ' ', sha1('abc\nabc\nab'), ' ', sha1("abc\nabc\nab"), "\n";
var_dump(hash_init('md5') instanceof HashContext);
$c = hash_init('sha1'); echo hash_final($c, true) === sha1('', true) ? "raw ok\n" : "raw bad\n";
unlink($data);
