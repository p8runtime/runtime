<?php
class CS extends SplObjectStorage { public $n = 0; }
class H {
    protected $scope;
    function go() {
        $this->scope = new CS;
        $c = function () {};
        $this->scope[$c] = $this;
        $this->scope->n++;
        var_dump(count($this->scope), isset($this->scope[$c]), $this->scope[$c] === $this);
        unset($this->scope[$c]);
        var_dump(count($this->scope));
    }
}
(new H)->go();
$o = new stdClass;
$h = new stdClass;
$h->s = new SplObjectStorage;
$h->s[$o] = 3;
var_dump($h->s[$o]);
$h->a = ['x' => new SplObjectStorage];
$h->a['x'][$o] = 4;
var_dump($h->a['x'][$o]);
$h->arr = [];
try { $h->arr[$o] = 1; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
