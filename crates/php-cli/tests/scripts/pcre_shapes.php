<?php
// The pattern shapes the preg_* fast paths touch: literal prefixes and
// first bytes (memchr/memmem) in long subjects, at offsets, caseless and
// UTF-8 ones; greedy classes over ASCII and multi-byte text; capture
// arrays (whole-subject groups, named groups, unmatched ones, offsets);
// matches reused across loops (match_all, replace, split, grep, callbacks
// that call preg_* themselves); pcre.* settings changed mid-script.
$long = str_repeat('The quick brown fox jumps over the lazy dog. ', 40) . 'id=12345 end';
var_dump(preg_match('/id=(\d+)/', $long, $m), $m);
var_dump(preg_match('/zzz/', $long), preg_match('/lazy dog\. id/', $long, $m), $m);
var_dump(preg_match('/dog/', $long, $m, PREG_OFFSET_CAPTURE, 100), $m);
var_dump(preg_match('/ID=(\d+)/i', $long, $m), $m);
var_dump(preg_match('/(?:ab|cd)x/', 'zzcdx'), preg_match('/[xyz]q/', 'aaazq'), preg_match('/[xy]q/', 'aaayq'));
var_dump(preg_match('/ab/', 'xxab', $m, 0, 3), preg_match('/ab/', 'xxab', $m, 0, 2), $m);
var_dump(preg_match('/^\d+$/', '123456', $m), $m, preg_match('/^\d+$/', '12a456'));
var_dump(preg_match('/^(\w+)@(\w+)\.(\w+)$/', 'user@example.com', $m), $m);
var_dump(preg_match('/^(?<y>\d{4})-(?<m>\d\d)-(?<d>\d\d)$/', '2024-06-07', $m), $m);
var_dump(preg_match('/(a)(b)?(c)/', 'ac', $m), $m);
var_dump(preg_match('/(a)(b)?(c)/', 'ac', $m, PREG_UNMATCHED_AS_NULL), $m);
var_dump(preg_match('/(?<x>a)|(?<x>b)/J', 'b', $m), $m);
var_dump(preg_match('/^[\p{L} ]+$/u', 'héllo wörld', $m), $m);
var_dump(preg_match('/ö(r)/u', 'héllo wörld', $m, PREG_OFFSET_CAPTURE), $m);
var_dump(preg_match('/wör/u', 'héllo wörld wör', $m, 0, 12), $m);
var_dump(preg_match('/[a-zé]+/u', 'ÉCOLE école', $m), $m);
var_dump(preg_match('/[a-zé]+/iu', 'ÉCOLE école', $m), $m);
var_dump(preg_match('/\w+/', "caf\xc3\xa9 au lait", $m), $m);
var_dump(preg_match('/x/u', "bad \xff utf"), preg_last_error(), preg_last_error_msg());
var_dump(preg_match('/x/', 'x'), preg_last_error());
var_dump(preg_match_all('/\w+/', 'one two three four five', $m), $m);
var_dump(preg_match_all('/(\d)(x)?/', '1x2 3x', $m, PREG_SET_ORDER), $m);
var_dump(preg_match_all('/a*/', 'baaac', $m), $m);
var_dump(preg_replace('/\s+/', ' ', "a  b \t c\n d"), preg_replace('/o/', '0', 'foo boo'), preg_replace('/x*/', '-', 'abc'));
var_dump(preg_replace('/(\w+) (\w+)/', '$2 ${1}!', 'hello world'), preg_replace('/a/', 'b', 'aaa', 2, $count), $count);
var_dump(preg_replace_callback('/\d+/', fn($m) => $m[0] * 2, 'a1 b22 c333'));
var_dump(preg_replace_callback('/\w+/', fn($m) => preg_replace('/[aeiou]/', '*', $m[0]) . (preg_match('/^\w+$/', $m[0], $mm) ? $mm[0][0] : ''), 'nested calls here'));
var_dump(preg_split('/[\s,]+/', 'a, b  c,d'), preg_split('//', 'abc', -1, PREG_SPLIT_NO_EMPTY), preg_split('/(-)/', 'a-b-c', -1, PREG_SPLIT_DELIM_CAPTURE));
var_dump(preg_grep('/^\d+$/', ['1', 'a', '22', 'b3']), preg_grep('/^\d+$/', ['1', 'a', '22'], PREG_GREP_INVERT));
var_dump(preg_match('#^/posts/(?P<id>[^/]+)/comments$#sDu', '/posts/42/comments', $m), $m);
$m = 'kept?';
var_dump(preg_match('/(/', 'x', $m), $m);
$m = 'kept?';
var_dump(preg_match('/a/', 'b', $m), $m);
ini_set('pcre.backtrack_limit', '10');
var_dump(preg_match('/(\w+\s?)+$/', str_repeat('word ', 10) . '!'), preg_last_error_msg());
ini_set('pcre.backtrack_limit', '1000000');
var_dump(preg_match('/(\w+\s?)+$/', str_repeat('word ', 3) . '!'), preg_last_error_msg());
ini_set('pcre.jit', '0');
ini_set('pcre.recursion_limit', '5');
var_dump(preg_match('/(a|b)*c/', str_repeat('ab', 50) . 'c'), preg_last_error_msg());
ini_set('pcre.recursion_limit', '100000');
var_dump(preg_match('/(a|b)*c/', str_repeat('ab', 50) . 'c'), preg_last_error_msg());
class S { function __toString(): string { return 'id=7'; } }
var_dump(preg_match('/id=(\d)/', new S, $m), $m, preg_replace('/\d/', 'N', [new S, 'x1']));
