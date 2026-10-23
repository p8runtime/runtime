<?php
interface Shape { const SIDES = 0; public function area(): float; }
abstract class Base implements Shape {
    public static int $count = 0;
    protected string $name;
    public function __construct(string $name) { $this->name = $name; static::$count++; self::$count += 0; }
    public function describe() { return $this->name . " area " . $this->area() . " sides " . static::SIDES; }
    abstract public function area(): float;
}
class Rect extends Base {
    const SIDES = 4;
    private $w; private $h;
    public function __construct($w, $h) { parent::__construct("rect"); $this->w = $w; $this->h = $h; }
    public function area(): float { return $this->w * $this->h; }
}
final class Sq extends Rect {
    public function __construct($s) { parent::__construct($s, $s); $this->name = "square"; }
}
$r = new Rect(2, 3);
echo $r->describe(), "\n";
$s = new Sq(4);
echo $s->describe(), "\n";
echo Base::$count, " ", Rect::SIDES, " ", Shape::SIDES, "\n";
var_dump($s instanceof Shape, $s instanceof Rect, $r instanceof Sq);
var_dump($r);
print_r($s); echo "\n";
$c = clone $r;
var_dump($c == $r, $c === $r, $r === $r);
$o = new stdClass;
$o->a = 1; $o->b = [1,2]; $o->b[] = 3; $o->a += 5; $o->a++;
var_dump($o, isset($o->a), isset($o->zz), $o->zz ?? "dflt");
unset($o->a);
var_dump($o);
class P { public $x = 1; public $arr = []; private $secret = 's'; public function get() { return $this->secret; } }
$p = new P;
$p->arr['k']['j'] = 5;
$p->arr['k']['j'] += 2;
var_export($p); echo "\n";
echo $p->get(), "\n";
$p->dyn = 3;
echo Rect::class, "\n";
