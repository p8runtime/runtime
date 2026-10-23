<?php error_reporting(E_ALL & ~E_NOTICE);
class P { public $a = 1; public $b = [2]; }
$vals = [null, true, false, 0, 1, -1, 1.5, NAN, "", "0", "1", "abc", "1e3", " 1", "1 ", "10", "9", [], [1], [1, 2], ['a' => 1], [[2]], new P, new stdClass];
$i = 0;
foreach ($vals as $x) foreach ($vals as $y) {
  echo $i++ % 24 ? '' : "\n", ($x == $y ? '=' : '.'), ($x === $y ? 'I' : '.'), ($x < $y ? '<' : '.'), ($x > $y ? '>' : '.'), match ($x <=> $y) { -1 => '-', 0 => '0', 1 => '+' }, ' ';
}
echo "\n";
$p1 = new P; $p2 = new P; var_dump($p1 == $p2, $p1 === $p2, $p1 === $p1); $p2->a = 2; var_dump($p1 < $p2);
var_dump(in_array([1], [[1]]), in_array("1e1", ["10"]), array_search("abc", [0, "abc"]), in_array(null, [0], true));
var_dump("abc" == 0, "1" == "01", "10" == "1e1", 100 == "1e2", null == false, [] == false, "abc" <=> "abd");
