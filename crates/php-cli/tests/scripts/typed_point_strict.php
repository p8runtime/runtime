<?php
declare(strict_types=1);
// typed_point_forms.php's int-for-float conversions under strict_types:
// the one conversion strict mode makes, and what it refuses.

class Point {
    public function __construct(private float $x = 0.0, private float $y = 0.0) {}
    public function x(): float { return $this->x; }
    public function add(Point $o): static { $this->x += $o->x(); $this->y += $o->x(); return $this; }
}
$sum = new Point();
for ($i = 0; $i < 100; $i++) {
    $sum->add(new Point($i % 7, $i % 11));
}
var_dump($sum->x());

function f_float(float $x) { var_dump($x); }
function f_union(float|string $x) { var_dump($x); }
function f_bool(float|bool $x) { var_dump($x); }
function f_var(float ...$xs) { var_dump($xs); }
function f_ret(int $i): float { return $i; }
function f_two(float $a, float $b) { var_dump($a + $b); }
f_float(1); f_union(2); f_bool(3); f_var(4, 5.5); var_dump(f_ret(6));
f_two(1, 2);
foreach (['f_float' => ["1"], 'f_bool' => ["x"], 'f_two' => [1, "2"], 'f_var' => [1, true]] as $f => $args) {
    try {
        $f(...$args);
    } catch (TypeError $e) {
        echo get_class($e), ": ", preg_replace('/, called in .*/', '', $e->getMessage()), "\n";
    }
}
