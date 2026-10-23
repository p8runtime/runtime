<?php
echo implode(' ', hash_algos()), "\n";
$inputs = ['', 'a', 'abc', 'The quick brown fox jumps over the lazy dog'];
foreach ([3, 15, 16, 17, 31, 32, 33, 55, 56, 63, 64, 65, 117, 118, 119, 127, 128, 129, 200, 1000] as $n) $inputs[] = substr(str_repeat('0123456789abcdefghij', 60), 0, $n);
foreach (hash_algos() as $a) {
    $out = [];
    foreach ($inputs as $in) $out[] = hash($a, $in);
    echo $a, ' ', implode(' ', $out), "\n";
}
foreach (['murmur3a', 'murmur3c', 'murmur3f', 'xxh32', 'xxh64', 'xxh3', 'xxh128'] as $a) echo "$a seed ", hash($a, 'seeded data', false, ['seed' => 42]), "\n";
foreach (hash_hmac_algos() as $a) echo "hmac $a ", hash_hmac($a, 'data', 'key'), "\n";
