<?php
$data = ['name' => 'Ann "A" <b>', 'age' => 30, 'tags' => ['x', 'y'], 'ratio' => 0.1, 'whole' => 2.0, 'ok' => true, 'none' => null, 'url' => 'a/b', 'uni' => "é→", 'empty' => [], 'obj' => new stdClass];
echo json_encode($data), "\n";
echo json_encode($data, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE), "\n";
echo json_encode([1, 2, 3]), json_encode([1 => 'a', 2 => 'b']), json_encode([3 => 1], JSON_FORCE_OBJECT), json_encode("3.5", JSON_NUMERIC_CHECK), json_encode(1.0, JSON_PRESERVE_ZERO_FRACTION), "\n";
echo json_encode("<a href='x'>&</a>", JSON_HEX_TAG | JSON_HEX_APOS | JSON_HEX_QUOT | JSON_HEX_AMP), "\n";
class P implements JsonSerializable { public $pub = 1; protected $prot = 2; private $priv = 3; function jsonSerialize(): mixed { return ['custom' => $this->priv]; } }
class Q { public $a = 1; protected $b = 2; public $c; }
echo json_encode(new P), json_encode(new Q), "\n";
var_dump(json_decode('{"a":1,"b":[1,2,{"c":null}],"d":1.5e3,"e":"xé\n","f":true}'));
var_dump(json_decode('{"a":{"b":2}}', true), json_decode('[1,"2",3.0]'), json_decode('12345678901234567890'), json_decode('12345678901234567890', false, 512, JSON_BIGINT_AS_STRING));
var_dump(json_decode('{bad}'), json_last_error(), json_last_error_msg());
var_dump(json_decode('"ok"'), json_last_error_msg());
try { json_decode('{', false, 512, JSON_THROW_ON_ERROR); } catch (JsonException $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump(json_encode(NAN), json_last_error_msg(), json_encode("\xff"), json_last_error());
var_dump(json_validate('[1,2]'), json_validate('{x'));
$v = 'outer'; $arr = ['v' => 'from extract', 'w' => 'new'];
function f() { $a = 1; $b = 2; return compact('a', 'b'); }
print_r(f());
function g($x, $y = 5) { return [func_num_args(), func_get_args(), func_get_arg(0)]; }
print_r(g(1)); print_r(g(1, 2, 3));
extract(['v' => 'extracted', 'z' => 'zz']); echo $v, $z, "\n";
$s = "123"; settype($s, "int"); var_dump($s); settype($s, "array"); var_dump($s);
$p = ['a' => 1, 'b' => 2, 'c' => 3];
echo current($p), key($p), next($p), next($p), var_export(next($p), true), reset($p), end($p), prev($p), key($p), "\n";
$sp = [1, 2, 3, 4, 5]; $removed = array_splice($sp, 1, 2, ['x', 'y', 'z']); print_r($sp); print_r($removed);
print_r(array_merge_recursive(['a' => [1], 'b' => 2], ['a' => [3], 'b' => 4]));
print_r(array_replace_recursive(['a' => ['x' => 1, 'y' => 2]], ['a' => ['y' => 3]]));
$rows = [['id' => 1, 'n' => 'a'], ['id' => 2, 'n' => 'b']]; print_r(array_column($rows, 'n', 'id')); print_r(array_column($rows, null, 'n'));
print_r(array_diff_assoc(['a' => 1, 'b' => 2, 0 => 3], ['a' => 1, 'b' => 3])); print_r(array_intersect_assoc(['a' => 'g', 'b' => 'r'], ['a' => 'g', 'b' => 'x']));
$nested = [1, [2, 3]]; array_walk_recursive($nested, function (&$v, $k) { $v *= 10; }); print_r($nested);
function h() { $local = 'L'; return array_keys(get_defined_vars()); } print_r(h());
