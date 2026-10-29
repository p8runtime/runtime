<?php
// `$obj->p[k] = v` where the property holds an ArrayAccess object: its
// offsetSet (Requests' Response::$headers is written this way).
class Dict implements ArrayAccess {
    protected $data = [];
    public function offsetExists($o): bool { return isset($this->data[strtolower($o)]); }
    public function offsetGet($o): mixed { return $this->data[strtolower($o)] ?? null; }
    public function offsetSet($o, $v): void {
        echo "offsetSet(", var_export($o, true), ", ", var_export($v, true), ")\n";
        if ($o === null) { $this->data[] = $v; return; }
        $this->data[strtolower($o)][] = $v;
    }
    public function offsetUnset($o): void { unset($this->data[strtolower($o)]); }
    public function all() { return $this->data; }
}
class Response { public $headers = []; private $meta; public $nested = []; public $plain;
    public function __construct() { $this->headers = new Dict(); $this->meta = new Dict(); $this->nested = ['d' => new Dict()]; }
    public function setMeta($k, $v) { $this->meta[$k] = $v; return $this->meta->all(); }
}
$r = new Response;
foreach (["Content-Type: text/html", "X-A: 1", "content-type: more"] as $h) {
    [$k, $v] = explode(':', $h, 2);
    $r->headers[$k] = trim($v);
}
$r->headers[] = 'appended';
var_dump($r->headers->all());
var_dump($r->setMeta('Key', 'v'));
$r->nested['d']['K'] = 'deep';
var_dump($r->nested['d']->all());
class Flat extends ArrayObject {}
$o = new stdClass; $o->d = new Flat; $o->d["x"] = "a"; $o->d["x"] .= "b";
var_dump($o->d->getArrayCopy());
$r->plain = [];
$r->plain['a']['b'] = 1;
var_dump($r->plain);
