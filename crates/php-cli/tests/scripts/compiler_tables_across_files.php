<?php
// Each file is compiled with name tables (constants, variables, call
// sites, class names) that an earlier file's compile left emptied on
// the thread: what a file means must not depend on what was compiled
// before it. The files use the same names in other orders, with other
// values, and more of them than the first had room for.
function seen(string $what) { echo $what, "\n"; return strlen($what); }
class Alpha { const N = 'alpha'; static function make() { return new static; } function name() { return static::N; } }
class Beta extends Alpha { const N = 'beta'; }

require __DIR__ . '/compiler_tables_across_files/one.inc';
echo one('x'), "\n";
require __DIR__ . '/compiler_tables_across_files/two.inc';
echo two('y'), "\n";
require __DIR__ . '/compiler_tables_across_files/three.inc';
echo three(), "\n";
// The same file's code again, and new code after all three.
echo one('z'), "\n";
echo eval('$q = "gamma"; $p = "alpha"; return seen($p . "-" . $q) + 1;'), "\n";
echo eval('return Beta::make()->name() . "/" . Alpha::make()->name() . "/" . strtoupper("delta");'), "\n";
