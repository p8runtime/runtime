<?php
// Hash tables at their edges: 7, 8, 9 and more keys (a small table, the
// first chained one, growth), packed lists turning into hashes, deletes
// (in the middle, at the ends, all of them, enough to compact), re-adds,
// int and string keys mixed, iteration order, foreach while writing,
// copies written apart, and the internal pointer across growth.
function keys($a) { return implode(',', array_map(fn($k) => is_int($k) ? $k : "'$k'", array_keys($a))); }
foreach ([7, 8, 9, 16, 17, 33, 100] as $n) {
    $a = [];
    for ($i = 0; $i < $n; $i++) $a["k$i"] = $i;
    $b = $a;
    unset($a['k0'], $a["k" . ($n - 1)], $a['k3']);
    $a['k0'] = 'again';
    $a[] = 'pushed';
    $a[-5] = 'neg';
    $b['k1'] = 'copy';
    echo "$n: ", count($a), ' ', keys($a), "\n";
    echo "   ", count($b), ' ', $b['k1'], ' ', $a['k1'], ' ', isset($a['k3']) ? 'set' : 'unset', ' ', $a['k2'] ?? '-', "\n";
}
// Packed to hash at each size.
foreach ([1, 7, 8, 9, 20] as $n) {
    $l = range(0, $n - 1);
    $l['s'] = 's';
    $l[$n + 5] = 'gap';
    $l[] = 'next';
    unset($l[0]);
    echo "packed $n: ", keys($l), ' ', json_encode(array_values($l)), "\n";
}
// Delete nearly everything (compaction), then grow again.
$c = [];
for ($i = 0; $i < 50; $i++) $c[$i * 3] = "v$i";
for ($i = 0; $i < 48; $i++) unset($c[$i * 3]);
echo keys($c), "\n";
for ($i = 0; $i < 20; $i++) $c["n$i"] = $i;
echo count($c), ' ', keys($c), "\n";
foreach ($c as $k => $v) { if ($k === 'n3') { unset($c['n4']); $c['added'] = 1; } }
echo keys($c), "\n";
// Empty out entirely, then reuse.
$e = ['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4, 'e' => 5, 'f' => 6, 'g' => 7, 'h' => 8, 'i' => 9];
foreach (array_keys($e) as $k) unset($e[$k]);
var_dump($e, count($e));
$e['z'] = 26;
$e[] = 'int';
var_dump($e);
// The internal pointer across growth and copies.
$p = ['x' => 1, 'y' => 2, 'z' => 3];
next($p);
for ($i = 0; $i < 20; $i++) $p["g$i"] = $i;
echo key($p), ' ', current($p), "\n";
$q = $p;
end($q);
echo key($q), ' ', key($p), "\n";
// Same hash slot, many keys (ints spaced by the table size).
$s = [];
for ($i = 0; $i < 40; $i++) $s[$i * 1024] = $i;
unset($s[5 * 1024], $s[0]);
echo count($s), ' ', $s[39 * 1024], ' ', array_key_first($s), ' ', array_key_last($s), "\n";
// Searching and sorting keep working on chained tables.
$w = [];
foreach (str_split('thequickbrownfoxjumpsoverthelazydog') as $i => $ch) $w[$ch . $i] = $i;
ksort($w);
echo keys(array_slice($w, 0, 10)), ' ', array_search(7, $w), ' ', in_array(34, $w) ? 'y' : 'n', "\n";
var_dump(array_flip(['a', 'b', 'c', 'a']), array_unique([3, 1, 3, 2, 1]), array_combine(range(1, 9), str_split('abcdefghi')));
