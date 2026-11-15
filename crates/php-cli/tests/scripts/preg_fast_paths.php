<?php
// The regex engine's fast paths against PHP 8.6 (PCRE2): runs of literal
// bytes matched as one comparison, short subjects searched at each
// candidate start, short haystacks searched by their first byte, and the
// pattern cache that finds recent pattern strings by address.

// A value in one short line: strings quoted (hex when not printable).
function d($v) {
    if (is_array($v)) {
        $parts = [];
        foreach ($v as $k => $x) {
            $parts[] = (is_int($k) ? $k : "'$k'") . ':' . d($x);
        }
        return '[' . implode(',', $parts) . ']';
    }
    if (is_string($v)) {
        return preg_match('/^[\x20-\x7e]*$/', $v) ? "'$v'" : 'x' . bin2hex($v);
    }
    return var_export($v, true);
}

function show($label, $v) {
    echo $label, ' => ', d($v), ' err=', preg_last_error(), "\n";
}

$patterns = [
    // Literal runs: plain, around groups, inside alternations and loops,
    // jumps into the middle of a run, and the end of the subject inside one.
    '#^/users/(\d+)/posts/(\d+)(?:\?page=(\d+))?$#',
    '/abc/', '/abcd$/', '/^abc/', '/(?:ab|b)cd/', '/x(?:abc)+y/', '/(abc)?abd/', '/a(?:bc|bd)e/',
    '/abc/i', '/ABC/i', '/hello world/x', '/ab c/x', '/(?i)ab(?-i)cd/',
    '/caf\x{e9}s/u', '/€uro/u', '/ab€/u', '/\xe9t\xe9/', '/(ab)\1/', '/(?<n>ab)cd(?P=n)/',
    '/(?<=ab)cd/', '/(?<!ab)cd/', '/ab(?=cd)/', '/ab(?!cd)/', '/\babc\b/', '/ab\Bcd/',
    '/a.c/s', '/a.c/', '/^ab$/m', '/ab$/D', '/ab$/', '/a+?bc/U', '/(*ANYCRLF)ab$/m',
    // Short subjects searched at each candidate start (one, two, three or
    // a text prefix of first bytes), matching, failing and failing late.
    '/\{\{\s*(\w+)\s*\}\}/', '/\{\{(\w+)\}\}/', '/[ab]c+/', '/[abc]d/', '/(?:xy|z)w/', '/q[^q]*q/',
    '/a\w*z/', '/za|zb/', '/\[(\w+)(?:\s+(\w+)="([^"]*)")*\]/', '/<(\w+)[^>]*>/', '/&(?:amp|lt|gt);/',
    '/\Gab/', '/ab|/', '/x*/', '/(?=ab)/', '/\d{2,}/', '/[aeiou]{2}/',
    // Backtracking that grows fast on 32 to 127 bytes: past the machine's
    // first try, settled by the DFA.
    '/(a|aa)*([bcz])/', '/(x+x+)+(q)/', '/(\w+)\s(\w+)/',
];
$subjects = [
    '', 'a', 'ab', 'abc', 'abcd', 'xabcd', 'xabcabcy', 'abd', 'abde', 'abce', 'ababcd', 'ab cd', 'ABC abc AbC',
    '/users/42/posts/126?page=6', '/users/42/posts/126', '/users/42/posts/126?page=', '/users/x/posts/1',
    'Hello {{ name }}, you have {{ count }} items', '{{ a }}{{b}}{{ c', '{ {x}} {{y}}', 'no tags here at all',
    'cafés', 'caf' . "\xe9" . 's', "\xe9t\xe9", '€uro', 'ab€', "ab\xff", "\xc3(",
    "ab\n", "ab\r\n", "x\nab\ny", 'zzzzzzzzzzzzza', 'ab ab abcd', '[tag a="1" b="2"]', '<p class="x">hi</p>', 'a &amp; b &lt; c',
    str_repeat('ab', 31) . 'cd', str_repeat('x', 60) . '{{ end }}', str_repeat('x', 63) . 'abc', str_repeat('x', 64) . 'abc',
    str_repeat('x', 125) . 'qabq', str_repeat('x', 126) . 'qabq', str_repeat('q', 127) . 'q', str_repeat('y', 129) . '{{ z }}',
    str_repeat('a', 40) . 'z', str_repeat('[x]', 20), '12 345 6789',
];

// One line per pattern and subject: preg_match, preg_match_all, and
// (on a match) the same match with offsets from offset 1, preg_replace
// and preg_split. Errors show as their code.
foreach ($patterns as $p) {
    echo "$p\n";
    foreach ($subjects as $i => $s) {
        $r = @preg_match($p, $s, $m);
        $e = preg_last_error();
        $line = [$i, $r === false ? "E$e" : $m];
        if ($r !== false) {
            $line[] = @preg_match_all($p, $s, $all, PREG_SET_ORDER) === false ? 'E' . preg_last_error() : $all;
        }
        if ($r) {
            $line[] = @preg_match($p, $s, $o, PREG_OFFSET_CAPTURE | PREG_UNMATCHED_AS_NULL, 1) === false ? 'E' . preg_last_error() : $o;
            $line[] = @preg_replace($p, '<$0|$1>', $s);
            $line[] = @preg_split($p, $s, -1, PREG_SPLIT_DELIM_CAPTURE | PREG_SPLIT_NO_EMPTY);
        }
        echo ' ', d($line), "\n";
    }
}

// Offsets inside and past a literal run.
foreach ([0, 1, 2, 3, 5, 26, 27] as $at) {
    show("offset $at", [preg_match('#/posts/(\d+)#', '/users/42/posts/126?page=6', $m, 0, $at), $m]);
    show("G offset $at", [preg_match('/\G\/posts/', '/users/42/posts/126', $m, 0, $at), $m]);
}

// The cache: more patterns in turn than it holds, patterns made at run
// time (same text, another string), and a bad pattern among them.
$cycle = [];
for ($k = 0; $k < 11; $k++) {
    $cycle[] = '/v' . $k . '(\d)/';
}
$cycle[] = '/unclosed(/';
$out = [];
for ($round = 0; $round < 3; $round++) {
    foreach ($cycle as $k => $p) {
        $made = implode('', str_split($p));
        $out[] = @preg_match($p, "v{$k}7 v{$k}8", $m) . ':' . ($m[1] ?? '-') . ':' . preg_last_error();
        $out[] = @preg_replace($made, '[$1]', "v{$k}5") . ':' . preg_last_error();
    }
}
echo implode(' ', $out), "\n";

// Limits on short subjects (without the JIT, where PHP has them).
ini_set('pcre.jit', '0');
ini_set('pcre.backtrack_limit', '100');
show('limit short', preg_match('/\{\{(\w+)+\}\}/', '{{' . str_repeat('a', 20) . '}'));
show('limit msg', preg_last_error_msg());
ini_set('pcre.backtrack_limit', '1000000');
ini_set('pcre.jit', '1');
show('limit restored', preg_match('/(a+)+b/', str_repeat('a', 20) . 'b'));
show('bad utf8', preg_match('/ab/u', "ab\xff"));
show('bad utf8 msg', preg_last_error_msg());
show('bad offset', preg_match('/b/u', "€b", $m, 0, 1));
show('named', [preg_match('/(?<year>\d{4})-(?<mon>\d\d)/', 'on 2026-11-14', $m), $m]);
show('callback', preg_replace_callback('/\{\{\s*(\w+)\s*\}\}/', fn($m) => strtoupper($m[1]), 'Hello {{ name }}, {{ x }}!'));
