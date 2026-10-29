<?php
// dir(): a Directory over opendir()'s handle (read, rewind, close); false
// for a missing directory; a closed handle is a TypeError.
$d = dir(__DIR__ . '/dir_function');
var_dump($d instanceof Directory, $d->path === __DIR__ . '/dir_function', is_resource($d->handle));
$n = [];
while (false !== ($e = $d->read())) { $n[] = $e; }
sort($n);
var_dump($n);
$d->rewind();
var_dump($d->read() !== false);
$d->close();
try { $d->read(); } catch (TypeError $t) { echo get_class($t), ': ', $t->getMessage(), "\n"; }
var_dump(@dir(__DIR__ . '/no-such-directory'));
