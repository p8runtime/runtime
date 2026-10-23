<?php
function f(int $x, ?string $s = null, float ...$fs): float { var_dump($x, $s, $fs); return $x; }
var_dump(f("5", 12, 1, "2.5"));
try { f("abc"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { f(null); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(f(1.0), f(true));
f(2.5);
function g(): int { return "7"; } var_dump(g());
function h(): string { return []; }
try { h(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
function none(): int { if (false) return 1; }
try { none(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
function v(): void { return; } var_dump(v());
function u(int|string $a, bool $b, iterable $it = []): ?array { var_dump($a, $b); return null; }
u(1.5, "yes"); u("x", 0);
class A { public int $i = 0; public ?float $f = null; public string $s = ''; public self $me; public array $arr = [];
  function make(): static { return new static; } function takes(self $o): self { return $o; } }
class B extends A {}
$a = new A;
$a->i = "42"; $a->f = 3; $a->s = 5.5;
var_dump($a->i, $a->f, $a->s);
try { $a->i = "nope"; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $a->arr = "x"; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $a->me = new stdClass; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$a->me = new B; echo get_class($a->me), "\n";
$a->i += 1.5; var_dump($a->i);
$a->i++; var_dump($a->i);
var_dump(get_class((new B)->make()), get_class($a->takes(new B)));
try { $a->takes(new stdClass); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$fn = function (int ...$n): int { return array_sum($n); }; var_dump($fn(1, "2"));
try { $fn(1, "x"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(array_map(fn(int $x): string => $x * 2, ["1", "2"]));
