<?php
// throw as an expression (PHP 8): match arms, ??, ?:, ||, arrow functions.
function m($x) { return match ($x) { 1 => [ 'after', 2 ], default => throw new \LogicException("Invalid: {$x}"), }; }
var_dump(m(1));
try { m(5); } catch (LogicException $e) { echo $e->getMessage(), "\n"; }
$n = null;
try { $y = $n ?? throw new Exception('null'); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
$f = fn() => throw new RuntimeException('arrow');
try { $f(); } catch (RuntimeException $e) { echo $e->getMessage(), "\n"; }
try { $v = false ?: throw new DomainException('elvis'); } catch (DomainException $e) { echo $e->getMessage(), "\n"; }
try { $v = false || throw new Exception('or'); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
echo "done\n";
