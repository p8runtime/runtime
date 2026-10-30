<?php
class C implements Countable { function count(): int { return 1; } }
function g() { yield 1; }
foreach ([[], [1], new ArrayIterator([]), new ArrayObject([]), g(), new C, new stdClass, 'abc', 1, null] as $v) {
    echo get_debug_type($v), ': ', var_export(is_iterable($v), true), ' ', var_export(is_countable($v), true), "\n";
}
$f = fopen('php://memory', 'r+');
var_dump(stream_set_chunk_size($f, 1024), stream_set_chunk_size($f, 4096));
try { stream_set_chunk_size($f, 0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
