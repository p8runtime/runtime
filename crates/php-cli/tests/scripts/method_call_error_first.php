<?php
// PHP finds a method call's method before it reads the arguments: a call
// on null, or of a method that is not there, is the error only. A
// variable that is not set is never read, so no "Undefined variable"
// warning comes before the error (#490).
class T {}
$n = null;
$t = new T;

// 5. Every argument a variable.
try { $n->m($a); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $t->nope($b); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $n->m($c, 1, $d); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// A computed argument beside the variable.
try { $n->m($e1, 1 + count([])); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $t->nope($f, strlen('x')); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// A warning handler would see nothing.
set_error_handler(function ($no, $text) { echo "handler: $text\n"; return true; });
try { $n->m($g); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $t->nope($h); } catch (Error $e) { echo $e->getMessage(), "\n"; }
