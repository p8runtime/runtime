<?php
$o = new stdclass; var_dump($o instanceof StdClass, get_class($o));
try { throw new exception("e"); } catch (EXCEPTION $e) { echo get_class($e), "\n"; }
class Foo {} class Bar extends foo implements countable { function count(): int { return 3; } } var_dump(count(new BAR), new bar instanceof FOO);
