<?php
// Constructor promotion declares the property and assigns it before the body.
class Point {
    public function __construct(public int $x = 0, protected ?int $y = null, private readonly string $label = 'p') {
        echo "body sees x={$this->x}\n";
    }
    public function describe(): string { return "{$this->label}({$this->x}, {$this->y})"; }
}
$p = new Point(3, 4);
echo $p->describe(), "\n";
echo (new Point(5))->x, "\n";
class Base { public function __construct(public $v = 1) {} }
class Child extends Base {}
var_dump((new Child(5))->v, (new Child)->v);
