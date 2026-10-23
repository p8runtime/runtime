<?php
// $a["key"] reads and writes: keys that are ints in disguise, misses,
// values of every kind, shared arrays.
$a = [];
$a["x"] = 1;
$a["10"] = "ten";
$a["010"] = "not ten";
$a["-5"] = 2.5;
$a["y"] = [1, 2];
$a["z"] = null;
$a["x"] = "again";
var_dump($a);
var_dump($a["10"], $a[10], $a["010"], $a["-5"], $a["y"], $a["z"]);
$b = $a;
$b["x"] = "changed";
var_dump($a["x"], $b["x"]);
var_dump($a["missing"]);
$h = [];
for ($i = 1; $i <= 20; $i++) {
    $h[dechex($i)] = $i;
}
$c = 0;
for ($i = 20; $i > 0; $i--) {
    if ($h[dechex($i)]) {
        $c++;
    }
}
echo $c, " ", $h["a"], " ", $h["14"], "\n";
$self = ["k" => 1];
$self["copy"] = $self;
var_dump($self);
