<?php
echo md5("hello"), " ", sha1("hello"), " ", crc32("hello"), " ", strlen(md5("x", true)), "\n";
foreach (['sha256', 'sha512', 'sha224', 'sha384', 'crc32b', 'adler32', 'fnv132', 'fnv1a32', 'fnv164', 'fnv1a64', 'joaat', 'sha512/256'] as $a) echo $a, "=", hash($a, "The quick brown fox"), "\n";
echo hash_hmac('sha256', 'data', 'key'), " ", hash_hmac('md5', 'data', str_repeat('k', 100)), "\n";
var_dump(hash_equals("abc", "abc"), hash_equals("abc", "abd"), in_array('sha256', hash_algos()));
try { hash('nope', 'x'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
mt_srand(42); echo mt_rand(), " ", mt_rand(), " ", mt_rand(1, 100), " ", mt_rand(1, 6), " ", rand(10, 20), "\n";
mt_srand(3); $a = range(1, 10); shuffle($a); echo implode(",", $a), " ", str_shuffle("abcdef"), " ", array_rand(['a' => 1, 'b' => 2, 'c' => 3]), " ", implode(",", array_rand(range(0, 9), 3)), " ", implode(",", array_rand(range(0, 9), 8)), "\n";
mt_srand(1); for ($i = 0; $i < 700; $i++) mt_rand(); echo mt_rand(), " ", mt_rand(0, PHP_INT_MAX), " ", mt_rand(-1000, 1000), "\n";
