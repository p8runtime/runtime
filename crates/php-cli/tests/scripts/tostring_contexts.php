<?php
// An object's __toString() in the engine's string contexts.
class Proc { public $n = 0; function __construct(public string $h = 'P') {} function __toString(): string { $this->n++; return $this->h; } }
$p = new Proc;
echo $p, "\n";
print $p; print "\n";
echo "in $p and {$p->h} string\n";
echo "x" . $p . "y", "|", $p . $p, "\n";
$s = 'a'; $s .= $p; echo $s, "\n";
$o = new Proc('left'); $o .= '!'; var_dump($o);
echo (string) $p, "|", strval($p), "|", strtoupper($p), "|", sprintf('[%s]', $p), "|", implode(',', [$p, $p]), "\n";
echo str_repeat($p, 2), " ", strlen($p), "\n";
echo "calls: ", $p->n, "\n";
class Thrower { function __toString(): string { throw new RuntimeException('no string'); } }
try { echo "a" . new Thrower; } catch (RuntimeException $e) { echo "caught: ", $e->getMessage(), "\n"; }
try { echo new Thrower; } catch (RuntimeException $e) { echo "caught echo: ", $e->getMessage(), "\n"; }
try { $x = "pre " . new stdClass; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { echo new stdClass; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $x = (string) new stdClass; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
class Nested { function __construct(private Proc $inner) {} function __toString(): string { return '<' . $this->inner . '>'; } }
echo new Nested(new Proc('deep')), "\n";
function f() { $o = new Proc('in function'); return "f: $o"; }
echo f(), "\n";
$arr = [new Proc('e1'), new Proc('e2')];
foreach ($arr as $item) { echo "item $item;"; }
echo "\n";
