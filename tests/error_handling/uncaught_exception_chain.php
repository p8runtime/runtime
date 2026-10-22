<?php
function f($a, $b) { throw new RuntimeException("inner", 3); }
class K { function m() { f(1, "a string longer than fifteen"); } static function s(array $x) { try { (new K)->m(); } catch (Exception $e) { throw new LogicException("outer", 0, $e); } } }
try { K::s([1]); } catch (Exception $e) { echo $e, "\n---\n", $e->getTraceAsString(), "\n---\n"; }
echo (new Exception()), "\n---\n";
ob_start(); echo "buffered\n";
K::s([1]);
