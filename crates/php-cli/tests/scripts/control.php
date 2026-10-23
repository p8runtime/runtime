<?php
declare(strict_types=1);
// Magic constants, switch, match.
echo __LINE__, "\n";
var_dump(__DIR__ . "/control.php" === __FILE__);
var_dump(__FUNCTION__, __METHOD__, __CLASS__, __NAMESPACE__, __TRAIT__);
function named() { return __FUNCTION__ . "/" . __METHOD__ . "@" . __LINE__; }
echo named(), "\n";
function sw($x) {
    $out = "";
    switch ($x) {
        case 1:
            $out .= "one ";
        case 2:
            $out .= "two ";
            break;
        case "3":
            $out .= "three ";
            break;
        default:
            $out .= "default ";
        case 4:
            $out .= "four ";
    }
    return $out;
}
foreach ([1, 2, 3, "3", 3.0, 4, 5, null, "1e0", true] as $i => $v) {
    echo $i, " => ", sw($v), "\n";
}
function trace($v) { echo "[$v]"; return $v; }
switch (2) {
    case trace(1): echo "no"; break;
    case trace(2): echo "yes"; break;
    case trace(3): echo "never"; break;
}
echo "\n";
for ($i = 0; $i < 5; $i++) {
    switch ($i) {
        case 1: continue 2;
        case 3: break 2;
    }
    echo $i;
}
echo "\n";
switch (1) { }
switch (5) { default: echo "only default\n"; }
function m($x) {
    return match ($x) {
        1, 2 => "small",
        "3" => "string three",
        3 => "int three",
        default => "other",
    };
}
foreach ([1, 2, 3, "3", 3.0, null] as $i => $v) {
    echo $i, " => ", m($v), "\n";
}
echo match (true) { 1 > 2 => "a", 2 > 1 => "b", default => "c" }, "\n";
echo match (trace(5)) { trace(4) => "four", trace(5) => "five", trace(6) => "six" }, "\n";
echo match (7) { 7 => match ("x") { "x" => "nested", default => "?" }, default => "!" }, "\n";
$v = 42;
echo match ($v) { 1 => "a" };
