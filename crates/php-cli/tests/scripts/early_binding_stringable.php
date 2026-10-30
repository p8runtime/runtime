<?php
// A class with __toString implements Stringable, so it is not bound early
// (nikic/php-parser's TokenPolyfill declares the same name conditionally first).
var_dump(class_exists('Plain', false), class_exists('Str', false));
if (PHP_VERSION_ID >= 80000) {
    class Poly { public $v = 'conditional'; }
    echo (new Poly)->v, "\n";
    return;
}
class Plain {}
class Str { public function __toString(): string { return 's'; } }
class Poly { public $v = 'fallback'; public function __toString(): string { return $this->v; } }
