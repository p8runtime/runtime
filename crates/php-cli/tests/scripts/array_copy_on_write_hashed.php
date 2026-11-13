<?php
// Copies of keyed arrays (copy on write): every key found in the copy, the
// copy's own writes (new keys go into the room after the last entry), the
// original untouched, order after unset and re-insert, references.

function keys_of(array $a): string {
    $out = [];
    foreach ($a as $k => $v) {
        $out[] = var_export($k, true) . '=' . (is_array($v) ? 'array(' . count($v) . ')' : var_export($v, true));
    }
    return implode(',', $out);
}

// 1. A large table (past eight entries): string, int and int-like keys.
$a = [];
for ($i = 0; $i < 20; $i++) {
    $a["k$i"] = $i;
}
$a[5] = 'five';
$a["05"] = 'zero five';
$a["-0"] = 'minus zero';
$a["9223372036854775808"] = 'too big';
$a[-3] = 'minus three';
$a["7"] = 'seven';
$b = $a;
$found = 0;
foreach (array_keys($a) as $k) {
    if (isset($b[$k]) && $b[$k] === $a[$k]) {
        $found++;
    }
}
echo "found in copy: $found of ", count($a), "\n";
$b["new"] = 'n';
$b[] = 'pushed';
$b["k3"] = 'changed';
for ($i = 100; $i < 140; $i++) {
    $b["x$i"] = $i;
}
echo "a: ", count($a), " ", $a["k3"], " ", isset($a["new"]) ? 'yes' : 'no', "\n";
echo "b: ", count($b), " ", $b["k3"], " ", $b["new"], " ", $b[8], " ", $b["x139"], "\n";
echo keys_of($a), "\n";
foreach (array_keys($b) as $k) {
    if (!array_key_exists($k, $b)) {
        echo "lost $k\n";
    }
}

// 2. Unset entries before the copy, re-inserted after it.
$c = [];
for ($i = 0; $i < 12; $i++) {
    $c["u$i"] = $i;
}
unset($c["u2"], $c["u5"], $c["u11"]);
$d = $c;
$d["u2"] = 'back';
$d["u11"] = 'back too';
unset($d["u0"]);
echo keys_of($c), "\n";
echo keys_of($d), "\n";
$e = $d;
for ($i = 0; $i < 9; $i++) {
    unset($e["u$i"]);
}
$e["u0"] = 'again';
echo keys_of($e), "\n";
echo keys_of($d), "\n";
var_dump(isset($d["u5"]), $d["u2"] ?? 'none', array_key_last($d), array_key_first($e));

// 3. A small table (up to eight entries).
$s = ["a" => 1, "b" => 2, 3 => "c"];
$t = $s;
$t["d"] = 4;
$t[] = "next";
unset($t["a"]);
echo keys_of($s), " | ", keys_of($t), "\n";

// 4. References: one still shared stays a reference in the copy; one whose
// variable is gone is a plain value in the copy.
$shared = 'shared';
$r = ["p" => 1, "q" => &$shared, "z" => 0];
for ($i = 0; $i < 10; $i++) {
    $r["f$i"] = $i;
}
$lonely = 'lonely';
$r["l"] = &$lonely;
unset($lonely);
$r2 = $r;
$r2["q"] = 'via copy';
$r2["l"] = 'only the copy';
echo $shared, " ", $r["q"], " ", $r["l"], " ", $r2["l"], "\n";

// 5. Nested arrays: a write through the copy separates only its path.
$n = ["one" => ["x" => 1, "y" => ["deep" => 1]], "two" => 2];
for ($i = 0; $i < 9; $i++) {
    $n["n$i"] = [$i];
}
$m = $n;
$m["one"]["y"]["deep"] = 2;
$m["n4"][] = 'more';
echo json_encode($n["one"]), " ", json_encode($m["one"]), " ", count($n["n4"]), " ", count($m["n4"]), "\n";

// 6. Iterated while written: foreach by value sees the array as it was,
// foreach by reference sees the writes.
$f = [];
for ($i = 0; $i < 10; $i++) {
    $f["e$i"] = $i;
}
$seen = [];
foreach ($f as $k => $v) {
    if ($k === "e1") {
        $f["late"] = 'L';
        unset($f["e5"]);
    }
    $seen[] = $k;
}
echo implode(',', $seen), "\n";
$g = $f;
$seen = [];
foreach ($g as $k => &$v) {
    if ($k === "e2") {
        $g["later"] = 'M';
        unset($g["e7"]);
    }
    $seen[] = $k;
    $v = is_int($v) ? $v * 10 : $v;
}
unset($v);
echo implode(',', $seen), "\n";
echo keys_of($f), "\n", keys_of($g), "\n";

// 7. Passed and returned.
function add_key(array $p, string $k): array {
    $p[$k] = strlen($k);
    return $p;
}
$h = [];
for ($i = 0; $i < 11; $i++) {
    $h["h$i"] = $i;
}
$h2 = add_key($h, "added");
echo count($h), " ", count($h2), " ", $h2["added"], " ", $h2["h10"], "\n";

// 8. Output of a copy.
$o = ["name" => "x", 10 => 1.5, "list" => [1, 2], "null" => null, "t" => true];
for ($i = 0; $i < 5; $i++) {
    $o["o$i"] = "v$i";
}
$o2 = $o;
$o2["extra"] = 'e';
unset($o2["o1"]);
var_dump($o2);
print_r($o);
echo "\n", json_encode($o2), "\n", serialize($o2), "\n";
echo "missing: ";
echo $o2["o1"] ?? 'none', "\n";
echo $o2["nope"];
echo "\n";
