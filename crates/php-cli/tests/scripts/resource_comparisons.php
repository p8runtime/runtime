<?php
// A resource compares as its number (zend_compare): two resources are ==
// only when they are the same resource, a resource against an int, a
// float or a string is its number against that as a number, an array or
// an object is greater (#536). Prints no resource number and no path.

// Every comparison of $x with $y and of $y with $x, as conditions.
function branches($x, $y): string
{
    return ($x == $y ? "==" : "..") . ($x != $y ? " !=" : " ..") . ($x < $y ? " <" : " .")
        . ($x <= $y ? " <=" : " ..") . ($x > $y ? " >" : " .") . ($x >= $y ? " >=" : " ..")
        . sprintf(" %2d", $x <=> $y) . " | "
        . ($y == $x ? "==" : "..") . ($y != $x ? " !=" : " ..") . ($y < $x ? " <" : " .")
        . ($y <= $x ? " <=" : " ..") . ($y > $x ? " >" : " .") . ($y >= $x ? " >=" : " ..")
        . sprintf(" %2d", $y <=> $x);
}

// The same as values.
function values($x, $y): string
{
    $r = [$x == $y, $x != $y, $x < $y, $x <= $y, $x > $y, $x >= $y, $x <=> $y,
        $y == $x, $y != $x, $y < $x, $y <= $x, $y > $x, $y >= $x, $y <=> $x];
    $t = fn (bool $is, string $op) => $is ? $op : str_repeat(".", strlen($op));
    return $t($r[0], "==") . " " . $t($r[1], "!=") . " " . $t($r[2], "<") . " " . $t($r[3], "<=")
        . " " . $t($r[4], ">") . " " . $t($r[5], ">=") . sprintf(" %2d", $r[6]) . " | "
        . $t($r[7], "==") . " " . $t($r[8], "!=") . " " . $t($r[9], "<") . " " . $t($r[10], "<=")
        . " " . $t($r[11], ">") . " " . $t($r[12], ">=") . sprintf(" %2d", $r[13]);
}

// The same through references.
function references(&$x, &$y): string
{
    return ($x == $y ? "==" : "..") . ($x != $y ? " !=" : " ..") . ($x < $y ? " <" : " .")
        . ($x <= $y ? " <=" : " ..") . ($x > $y ? " >" : " .") . ($x >= $y ? " >=" : " ..")
        . sprintf(" %2d", $x <=> $y) . " | "
        . ($y == $x ? "==" : "..") . ($y != $x ? " !=" : " ..") . ($y < $x ? " <" : " .")
        . ($y <= $x ? " <=" : " ..") . ($y > $x ? " >" : " .") . ($y >= $x ? " >=" : " ..")
        . sprintf(" %2d", $y <=> $x);
}

function row(string $left, string $right, $x, $y): void
{
    $b = branches($x, $y);
    $v = values($x, $y);
    $r = references($x, $y);
    echo str_pad("$left, $right", 34), $b, $b === $v && $b === $r ? "" : "  BUT as values: $v, through references: $r", "\n";
}

$a = fopen("php://memory", "r");
$b = fopen("php://memory", "r");
$same = $a;
$closed = fopen("php://memory", "r");
$closed2 = fopen("php://memory", "r");
fclose($closed);
fclose($closed2);
$context = stream_context_create();
$dir = opendir(__DIR__);
$process = proc_open("exit 0", [1 => ["pipe", "w"]], $out);
$late = fopen("php://memory", "r");
$resources = ["a" => $a, "b" => $b, "same as a" => $same, "closed" => $closed, "closed2" => $closed2,
    "context" => $context, "dir" => $dir, "pipe" => $out[1], "process" => $process, "late" => $late];
$name = fn ($r) => array_search($r, $resources, true);

echo "kinds:";
foreach ($resources as $r) {
    echo " ", gettype($r), is_resource($r) ? " " . get_resource_type($r) : "", ",";
}
echo "\n\n";

// Resource against resource: in the order they were made.
foreach ($resources as $left => $x) {
    foreach ($resources as $right => $y) {
        if ($left <= $right) {
            row($left, $right, $x, $y);
        }
    }
}
echo "\n";

// Resource against everything else: its number against that.
class Text
{
    public function __toString(): string
    {
        return "abc";
    }
}
enum Suit
{
    case Hearts;
}
enum Rank: int
{
    case One = 1;
}
foreach (["a" => $a, "closed" => $closed, "process" => $process] as $left => $x) {
    $n = (int) $x;
    $others = [
        "null" => null, "true" => true, "false" => false,
        "0" => 0, "-1" => -1, "n" => $n, "n - 1" => $n - 1, "n + 1" => $n + 1, "PHP_INT_MAX" => PHP_INT_MAX,
        "0.0" => 0.0, "n.0" => (float) $n, "n + 0.5" => $n + 0.5, "n - 0.5" => $n - 0.5, "NAN" => NAN, "INF" => INF, "-INF" => -INF,
        '""' => "", '"0"' => "0", '"abc"' => "abc", '"n"' => (string) $n, '" n"' => " $n", '"n "' => "$n ",
        '"nabc"' => "{$n}abc", '"n.0"' => "$n.0", '"n.5"' => "$n.5", '"1e3"' => "1e3", '"-1"' => "-1",
        '"Resource id #n"' => "Resource id #$n", '(string) itself' => (string) $x,
        "[]" => [], "[0]" => [0], "[n]" => [$n], "[itself]" => [$x],
        "stdClass" => new stdClass, "Text" => new Text, "closure" => fn () => null, "ArrayObject" => new ArrayObject([$x]),
        "enum case" => Suit::Hearts, "backed enum case" => Rank::One,
    ];
    foreach ($others as $right => $y) {
        row($left, $right, $x, $y);
    }
    echo "\n";
}

// Against what the code itself says.
var_dump($a == null, $a != null, null == $a, $a == true, $a == false, false == $a, $a < null, null < $a);
var_dump($a == 0, $a != 0, $a > 0, $a < 0, 0 < $a, $a <=> 0, $a == 1000000, $a < 1000000, 1000000 > $a, $a <=> 1000000);
var_dump($a == 0.0, $a > 0.5, $a < 1e9, 1e9 <=> $a, $a == "", $a == "0", $a == "abc", $a > "abc", "abc" < $a, $a <=> "abc");
var_dump($a == [], $a < [], [] > $a, $a <=> [], $closed == null, $closed == true, $closed > 0, $closed == "");
var_dump($a == $a, $a == $same, $a === $same, $a == $b, $a === $b, $a != $b, $a < $b, $b < $a, $a <=> $b, $b <=> $a, $closed == $closed2, $closed < $closed2);
echo "\n";

// switch compares with ==; match with ===.
function which($x, $a, $b): string
{
    switch ($x) {
        case $a:
            return "the first";
        case $b:
            return "the second";
        default:
            return "neither";
    }
}
echo which($a, $a, $b), ", ", which($b, $a, $b), ", ", which($same, $b, $a), ", ", which($closed2, $closed, $closed2), ", ",
    which($closed, $a, $b), ", ", which($late, $a, $b), ", ", which($process, $out[1], $process), ", ", which($a, (int) $a, $a), ", ",
    which((int) $b, $a, $b), ", ", which($a, null, true), "\n";
switch ($b) {
    case null:
        echo "null\n";
        break;
    case 0:
        echo "0\n";
        break;
    case "abc":
        echo "abc\n";
        break;
    case $a:
        echo "a\n";
        break;
    case $b:
        echo "b\n";
        break;
}
echo match ($b) {
    $a => "a",
    (int) $b => "b's number",
    $b => "b",
}, "\n";
// Cases the compiler knows, ints and then strings: a resource takes the
// case that is its number.
function by_int($x): int
{
    switch ($x) {
        case 1: return 1; case 2: return 2; case 3: return 3; case 4: return 4; case 5: return 5; case 6: return 6; case 7: return 7; case 8: return 8;
        case 9: return 9; case 10: return 10; case 11: return 11; case 12: return 12; case 13: return 13; case 14: return 14; case 15: return 15;
        case 16: return 16; case 17: return 17; case 18: return 18; case 19: return 19; case 20: return 20; case 21: return 21; case 22: return 22;
        case 23: return 23; case 24: return 24; case 25: return 25; case 26: return 26; case 27: return 27; case 28: return 28; case 29: return 29;
        case 30: return 30; case 31: return 31; case 32: return 32; case 33: return 33; case 34: return 34; case 35: return 35; case 36: return 36;
        case 37: return 37; case 38: return 38; case 39: return 39; case 40: return 40;
        default: return 0;
    }
}
function by_string($x): string
{
    switch ($x) {
        case "abc": return "abc"; case "": return "empty"; case "Resource": return "Resource";
        case "1": return "1"; case "2": return "2"; case "3": return "3"; case "4": return "4"; case "5": return "5"; case "6": return "6"; case "7": return "7";
        case "8": return "8"; case "9": return "9"; case "10": return "10"; case "11": return "11"; case "12": return "12"; case "13": return "13";
        case "14": return "14"; case "15": return "15"; case "16": return "16"; case "17": return "17"; case "18": return "18"; case "19": return "19";
        case "20": return "20"; case "21": return "21"; case "22": return "22"; case "23": return "23"; case "24": return "24"; case "25": return "25";
        case "26": return "26"; case "27": return "27"; case "28": return "28"; case "29": return "29"; case "30": return "30"; case "31": return "31";
        case "32": return "32"; case "33": return "33"; case "34": return "34"; case "35": return "35"; case "36": return "36"; case "37": return "37";
        case "38": return "38"; case "39": return "39"; case "40": return "40";
        default: return "none";
    }
}
var_dump((int) $late <= 40, by_int($a) === (int) $a, by_int($closed) === (int) $closed, by_int($process) === (int) $process, by_int($late) === (int) $late,
    by_string($a) === (string) (int) $a, by_string($closed2) === (string) (int) $closed2, by_string($late) === (string) (int) $late, by_int(null), by_string(null));
echo "\n";

// in_array, array_search, array_keys.
var_dump(in_array($b, [$a]), in_array($a, [$b, $same]), in_array($b, [$a, $closed, $late]), in_array($b, [$a], true), in_array($same, [$b, $a], true));
var_dump(in_array((int) $a, [$b, $a]), in_array((int) $a, [$b, $a], true), in_array($a, [(int) $b, (string) (int) $a]), in_array($a, ["abc", null, false, 0, [], "Resource"]), in_array($a, [true]), in_array("abc", [$a]));
var_dump(array_search($b, [1 => $a, 2 => $b]), array_search($b, [1 => $a, 2 => $b], true), array_search($a, [1 => $closed, 2 => $b]),
    array_search($closed2, [1 => $closed, 2 => $closed2]), array_search($out[1], $out), array_search($process, [$out[1], $a, $process]));
echo implode(",", array_keys([$a, $b, $same, $closed, (int) $a, (string) $a, null], $a)), " / ", implode(",", array_keys([$a, $b, $same, $closed, (int) $a], $a, true)), "\n";
var_dump([$a] == [$b], [$a] == [$same], [$a] === [$same], [$a] != [$b], [$a] < [$b], [$b] < [$a], [$a] <=> [$b], ["k" => $b] == ["k" => $b], [$a, $b] == [$a, $late]);
$box = function ($r) {
    $o = new stdClass;
    $o->r = $r;
    return $o;
};
var_dump($box($a) == $box($b), $box($a) == $box($same), $box($a) != $box($b), $box($closed) == $box($closed2));
echo "\n";

// sort, array_unique, max and min on resources.
$names = fn (array $list) => implode(", ", array_map($name, $list));
$list = [$late, $closed, $b, $process, $a, $context, $closed2];
sort($list);
echo "sort: ", $names($list), "\n";
rsort($list);
echo "rsort: ", $names($list), "\n";
$list = [$late, $closed, $b, $process, $a, $context, $closed2];
usort($list, fn ($x, $y) => $x <=> $y);
echo "usort: ", $names($list), "\n";
$keyed = ["late" => $late, "b" => $b, "a" => $a];
asort($keyed);
echo "asort: ", implode(", ", array_keys($keyed)), "\n";
$mixed = [$b, (int) $b - 1, $a, 0, $late];
sort($mixed);
echo "sort with ints: ", implode(", ", array_map(fn ($v) => is_int($v) ? ($v === 0 ? "0" : "b's number - 1") : $name($v), $mixed)), "\n";
echo "array_unique: ", $names(array_unique([$a, $b, $same, $closed, $b, $closed2, $closed])), " / ",
    $names(array_unique([$a, $b, $same, $closed, $b, $closed2, $closed], SORT_REGULAR)), "\n";
echo "max, min: ", $name(max($a, $b)), ", ", $name(max($b, $a)), ", ", $name(max([$closed, $late, $a])), ", ", $name(min($a, $b)), ", ",
    $name(min($b, $a)), ", ", $name(min([$closed, $late, $a])), ", ", $name(max($a, $same)), ", ", gettype(max($a, (int) $a)), ", ",
    gettype(max((int) $a, $a)), ", ", gettype(max($a, (int) $a + 1)), ", ", gettype(min($a, "abc")), ", ", gettype(max($a, [])), ", ", gettype(min($a, null)), "\n\n";

// The pattern that hung (php-src's proc_open_bug64438.phpt): the pipe that
// ended is looked up among the open ones with array_search. The child ends
// its second pipe, then waits for a line before it ends the first.
$child = proc_open('echo out; echo err 1>&2; exec 2>&-; read line; echo "got $line"', [["pipe", "r"], ["pipe", "w"], ["pipe", "w"]], $pipes);
var_dump($pipes[1] == $pipes[2], $pipes[1] < $pipes[2], array_search($pipes[2], $pipes), array_search($pipes[1], $pipes), in_array($child, $pipes));
$in = $pipes[0];
unset($pipes[0]);
$read = [1 => "", 2 => ""];
for ($round = 0; $pipes && $round < 100; $round++) {
    $r = $pipes;
    $w = $e = null;
    if (!stream_select($r, $w, $e, 10)) {
        echo "nothing to read\n";
        break;
    }
    foreach ($r as $pipe) {
        $type = array_search($pipe, $pipes);
        $data = fread($pipe, 8192);
        if ($data === false || $data === "" || feof($pipe)) {
            $read[$type] .= $data;
            echo "pipe $type ended\n";
            fclose($pipe);
            unset($pipes[$type]);
            if ($type === 2) {
                fwrite($in, "on\n");
            }
        } else {
            $read[$type] .= $data;
        }
    }
}
fclose($in);
foreach ($pipes as $type => $pipe) {
    echo "pipe $type left ", gettype($pipe), "\n";
}
var_dump($read, proc_close($child));
