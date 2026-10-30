<?php
// foreach into a property, an element or a static property (Kimai's bootstrap):
// the value is assigned before the key, as zend_compile_foreach does.
class Loader {
    public $cur; public $key; public $all = []; public static $last;
    function run(array $items) {
        foreach ($items as $this->key => $this->cur) { $this->all[] = "$this->key=$this->cur"; }
        foreach ($items as self::$last) {}
        return [$this->cur, $this->key, $this->all, self::$last];
    }
}
var_dump((new Loader)->run(['a' => 1, 'b' => 2]));
$arr = []; $o = new stdClass;
$i = 'pre'; foreach ([10, 20] as $i => $arr['v' . $i]) {}
foreach (['x' => 'y'] as $o->k => $o->v) {}
var_dump($arr, $o);
$m = ['p' => ['q' => 0]];
foreach ([5, 6] as $m['p']['q']) {}
var_dump($m);
