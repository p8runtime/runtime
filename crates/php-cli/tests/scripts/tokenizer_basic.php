<?php
$code = "<?php\nnamespace App\\Models;\n/** doc */\nclass A extends \\Base { public function f(&\$x, int ...\$r): ?string { return \$this?->name . \"v\$x\" ?? 1.5; } }\n?>\nhtml";
foreach (token_get_all($code) as $t) {
    echo is_array($t) ? token_name($t[0]) . " " . json_encode($t[1]) . " L$t[2]" : "'$t'", "\n";
}
var_dump(token_name(T_DOUBLE_COLON), token_name(T_PAAMAYIM_NEKUDOTAYIM), token_name(-1), TOKEN_PARSE);
$tokens = PhpToken::tokenize('<?php echo $a->class; // hi');
foreach ($tokens as $t) {
    echo $t->getTokenName(), " ", var_export($t->isIgnorable(), true), " pos=", $t->pos, " ", $t->is([T_ECHO, ';']) ? "match" : "-", "\n";
}
echo implode('|', $tokens), "\n";
class MyToken extends PhpToken { public function upper(): string { return strtoupper($this->text); } }
echo get_class(MyToken::tokenize('<?php x')[1]), " ", MyToken::tokenize('<?php x')[1]->upper(), "\n";
class S { function __toString(): string { return 's'; } }
var_dump(implode(',', [new S, 1, new S]));
