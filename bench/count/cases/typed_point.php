<?php
// perf/objects.php's loop: a typed value object made with ints for its promoted float properties,
// passed to a method taking the class, two float `+=` on its properties, `static` returned.
$n = (int) ($argv[1] ?? 1000);
class Point {
    public function __construct(private float $x = 0.0, private float $y = 0.0) {}
    public function x(): float { return $this->x; }
    public function y(): float { return $this->y; }
    public function add(Point $o): static { $this->x += $o->x(); $this->y += $o->y(); return $this; }
}
function run($n) { $sum = new Point(); for ($i = 0; $i < $n; $i++) { $sum->add(new Point($i % 7, $i % 11)); } return round($sum->x() + $sum->y()); }
echo run($n), "\n";
