<?php
// Objects with __toString() passed to builtins' string parameters become
// strings, as zend_parse_arg_str() converts them.
class Name {
    public function __construct(private string $v) {}
    public function __toString(): string { return $this->v; }
}
$n = new Name("Hello World");
var_dump(strlen($n));
var_dump(strtoupper($n));
var_dump(str_replace("World", "p8", $n));
var_dump(preg_match(new Name('/w(or)ld/i'), $n, $m), $m);
var_dump(explode(" ", $n));
try {
    strlen(new stdClass);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
