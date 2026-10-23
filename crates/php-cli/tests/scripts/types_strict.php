<?php
declare(strict_types=1);
function f(int $x): float { return $x; }
var_dump(f(5));
try { f("5"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
function g(): int { return "1"; }
try { g(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
class P { public int $i = 0; }
$p = new P; try { $p->i = "1"; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(strlen("abc"));
