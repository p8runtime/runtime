<?php
// Arguments a method takes by reference: variables, elements, properties,
// for instance, static, nullsafe and inherited methods; by value otherwise.
class Base { function setRef(&$x, $v = 'set') { $x = $v; } function append(array &$a, $v) { $a[] = $v; } }
class C extends Base {
    public $prop = 'p';
    public $list = [];
    public static $s = 's';
    function byVal($x) { $x = 'changed'; }
    function viaRef(&$x) { $this->setRef($x, 'via'); }
    static function sset(&$x) { $x = 'static'; }
    function __call($n, $a) { $a[0] = 'magic'; return $n; }
    function variadic(&...$xs) { foreach ($xs as &$x) { $x .= '!'; } }
}
$c = new C;
$v = 'orig'; $c->setRef($v); echo "var: $v\n";
$v = 'orig'; $c->byVal($v); echo "by value: $v\n";
$v = 'orig'; $c->viaRef($v); echo "chained: $v\n";
$a = ['k' => 'orig']; $c->setRef($a['k']); echo "element: {$a['k']}\n";
$a = []; $c->setRef($a['new']['deep']); echo "new element: ", json_encode($a), "\n";
$c->setRef($c->prop); echo "property: {$c->prop}\n";
$c->append($c->list, 1); $c->append($c->list, 2); echo "property array: ", json_encode($c->list), "\n";
$v = 'orig'; C::sset($v); echo "static: $v\n";
$v = 'orig'; $c::sset($v); echo "static via object: $v\n";
$a = ['x' => 'orig']; C::sset($a['x']); echo "static element: {$a['x']}\n";
$n = null; $v = 'orig'; $n?->setRef($v); echo "nullsafe null: $v\n";
$v = 'orig'; $c?->setRef($v); echo "nullsafe: $v\n";
$v = 'orig'; echo $c->missing($v), ": $v\n";
$p = 'a'; $q = 'b'; $c->variadic($p, $q); echo "variadic: $p $q\n";
$arr = [[]]; foreach ($arr as &$item) { $c->append($item, 'x'); } unset($item); echo "foreach ref: ", json_encode($arr), "\n";
$undefined_target = null; $c->setRef($fresh); var_dump($fresh);
$v = 'orig'; $c->byVal($a['nope'] ?? 'x'); echo "expression arg: ok\n";
class Tax {
    public $queries = [['terms' => ['slug'], 'field' => 'name']];
    function sql() { return $this->forQuery($this->queries); }
    function forQuery(&$query) { $out = []; foreach ($query as &$clause) { $out[] = $this->forClause($clause); } return $out; }
    function forClause(&$clause) { $this->clean($clause); return $clause['terms']; }
    function clean(&$q) { if ($q['field'] === 'id') return; $this->transform($q, 'id'); }
    function transform(&$q, $field) { $q['terms'] = []; $q['field'] = $field; }
}
$t = new Tax; echo json_encode($t->sql()), ' ', json_encode($t->queries), "\n";
