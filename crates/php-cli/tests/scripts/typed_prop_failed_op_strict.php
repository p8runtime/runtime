<?php
declare(strict_types=1);
// Under strict_types a typed property whose compound assignment's
// operation throws keeps its value (#572).

class C {
    public int $i = 7;
    public float $f = 2.0;
    public mixed $m = 5;
}
$c = new C();
try { $c->i /= 0; } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump($c->i);
try { $c->f %= 0; } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump($c->f);
try { $c->m <<= -1; } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump($c->m);
