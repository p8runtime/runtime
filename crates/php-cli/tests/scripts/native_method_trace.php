<?php
function f() { return intdiv(1, 0); }
try { f(); } catch (Error $e) { echo $e->getTraceAsString(), "\n"; }
$f = new Fiber(fn() => 1);
try { $f->getReturn(); } catch (Error $e) { echo $e->getTraceAsString(), "\n"; }
try { new ArrayIterator(1); } catch (Error $e) { echo $e->getTraceAsString(), "\n"; }
