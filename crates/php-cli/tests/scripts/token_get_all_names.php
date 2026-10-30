<?php
// Keywords read as names: after ->/?-> always (comments between too); with
// TOKEN_PARSE also after ::, function, const, enum case, named arguments (PHP-Parser's lexer).
$srcs = [
  '<?php class A { const AND = 1; public function or() { return $this->and() ?? static::list(); } } $x?->print; A::class; enum E: int { case Default = 1; case Or = 2; } fn() => match(1) { default => 1 };',
  '<?php class Q { function &xor() {} } f(array: 1, list: 2); switch ($a) { case FOO: break; } foreach ($a as list($b)) {} $o->class; $o::default; new class { function new() {} };',
  '<?php $a = $b and $c or $d xor $e; if ($x instanceof Y) {} /* c */ $y->/* mid */and; trait T { function print() {} }',
];
foreach ($srcs as $src) foreach ([0, TOKEN_PARSE] as $flags) {
    $out = [];
    foreach (token_get_all($src, $flags) as $t) { if (is_array($t) && $t[0] !== T_WHITESPACE) $out[] = token_name($t[0]) . ':' . $t[1]; }
    echo implode(' ', $out), "\n";
}
foreach (PhpToken::tokenize('<?php $a->or; A::list; class Z { function and() {} }', TOKEN_PARSE) as $t) if (!$t->isIgnorable()) echo $t->getTokenName(), ' ';
echo "\n";
