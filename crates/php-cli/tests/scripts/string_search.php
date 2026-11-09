<?php
// Substring search edge cases (#398): memmem/memchr search, the folded
// (ASCII-only) variants, and strtr's longest-match pairs. Recorded with PHP 8.6.

function show($label, $v) { echo $label, ': '; var_dump($v); }
function attempt($label, $f) {
    try { show($label, $f()); } catch (\Throwable $e) { echo $label, ': ', get_class($e), ': ', $e->getMessage(), "\n"; }
}

echo "-- empty needle\n";
show('strpos', strpos('abc', ''));
show('strpos off', strpos('abc', '', 2));
show('stripos', stripos('abc', '', 3));
show('strrpos', strrpos('abc', ''));
show('strrpos neg', strrpos('abc', '', -1));
show('strripos', strripos('abc', ''));
show('strstr', strstr('abc', ''));
show('stristr before', stristr('abc', '', true));
show('str_contains', str_contains('abc', ''));
show('str_contains empty', str_contains('', ''));
show('str_replace', str_replace('', 'x', 'abc'));
show('str_ireplace', str_ireplace('', 'x', 'abc'));
attempt('substr_count', fn() => substr_count('abc', ''));
attempt('explode', fn() => explode('', 'abc'));

echo "-- needle longer than haystack\n";
show('strpos', strpos('ab', 'abc'));
show('stripos', stripos('ab', 'ABC'));
show('strrpos', strrpos('ab', 'abc'));
show('strripos', strripos('ab', 'ABC'));
show('strstr', strstr('ab', 'abc'));
show('str_contains', str_contains('ab', 'abc'));
show('substr_count', substr_count('ab', 'abc'));
show('str_replace', str_replace('abc', 'x', 'ab'));
show('explode', explode('abc', 'ab'));
show('needle at end', strpos('xxab', 'ab'));
show('needle at end i', stripos('xxAB', 'ab'));
show('rneedle at start', strrpos('abxx', 'ab'));
show('rneedle at start i', strripos('ABxx', 'ab'));

echo "-- overlapping matches\n";
show('str_replace aaa', str_replace('aa', 'b', 'aaaaa'));
show('str_replace aba', str_replace('aba', 'X', 'abababa'));
show('str_ireplace aba', str_ireplace('ABA', 'X', 'AbAbAbA'));
show('substr_count aa', substr_count('aaaaa', 'aa'));
show('substr_count aba', substr_count('abababa', 'aba'));
show('substr_count off', substr_count('aaaaa', 'aa', 1));
show('substr_count len', substr_count('aaaaa', 'aa', 1, 3));
show('substr_count neg', substr_count('abababa', 'ab', -4));
$n = 0;
show('str_replace count', str_replace('aa', 'b', 'aaaaa', $n));
show('count', $n);
show('str_replace array', str_replace(['ab', 'b'], ['b', 'c'], 'abab'));
show('strrpos overlap', strrpos('aaaa', 'aa'));
show('strrpos off', strrpos('abcabc', 'abc', 1));
show('strrpos neg off', strrpos('abcabc', 'abc', -4));
show('strrpos neg off2', strrpos('abcabc', 'abc', -3));
show('strripos neg off', strripos('ABCabc', 'abc', -4));
show('stripos off', stripos('ABCabc', 'abc', 1));

echo "-- strtr pairs\n";
show('overlap keys', strtr('abcabc', ['ab' => 'X', 'abc' => 'Y', 'bc' => 'Z']));
show('prefix chain', strtr('aaaa', ['a' => '1', 'aa' => '2', 'aaa' => '3']));
show('no rescan', strtr('hi all', ['hi' => 'hello', 'hello' => 'bye', 'a' => 'A']));
show('swap', strtr('ab', ['a' => 'b', 'b' => 'a']));
show('empty key', strtr('abc', ['' => 'x', 'b' => 'B']));
show('only empty key', strtr('abc', ['' => 'x']));
show('empty subject', strtr('', ['a' => 'b', 'c' => 'd']));
show('empty array', strtr('abc', []));
show('longer than subject', strtr('ab', ['abc' => 'x', 'b' => 'B']));
show('int keys', strtr('a1b22c', [1 => 'one', 22 => 'twotwo', 2 => 'two']));
show('mixed keys', strtr('10 01 1', [10 => 'ten', '01' => 'oh-one', 1 => 'one']));
show('single pair', strtr('aaa', ['aa' => 'b']));
show('single byte pair', strtr('banana', ['a' => 'o']));
show('no match', strtr('xyz', ['a' => 'b', 'c' => 'd']));
show('values', strtr('a b c d', ['a' => 1, 'b' => 2.5, 'c' => true, 'd' => null]));
show('array value', strtr('a-a-b', ['a' => [1], 'b' => 'B']));
show('array value unused', strtr('xyz', ['a' => [1], 'b' => 'B']));
show('single array value', strtr('xyz', ['q' => [1]]));
show('binary', strtr("a\0b\0", ["\0" => '0', "b\0" => 'B']));
show('ties by length', strtr('abcd', ['abc' => '3', 'ab' => '2', 'bcd' => 'X', 'cd' => 'Y']));
show('three-arg', strtr('abc', 'ab', 'xyz'));

echo "-- case-insensitive, non-ASCII\n";
show('stripos ascii', stripos('Hello WORLD', 'world'));
show('stripos e-acute', stripos("caf\xc3\xa9 CAF\xc3\x89", "\xc3\x89"));
show('stripos latin1', stripos("\xc9t\xe9", "\xe9"));
show('stripos latin1 upper', stripos("\xe9t\xc9", "\xc9"));
show('strripos utf8', strripos("\xc3\xa9a\xc3\x89A", "\xc3\xa9A"));
show('stristr utf8', stristr("Stra\xc3\x9fe STRASSE", 'strasse'));
show('stristr sharp s', stristr("STRASSE stra\xc3\x9fe", "\xc3\x9f"));
show('str_ireplace utf8', str_ireplace("\xc3\xa9", 'e', "\xc3\x89t\xc3\xa9 \xc3\xa9T\xc3\x89"));
show('str_ireplace mixed', str_ireplace("A\xc3\xa9", '_', "a\xc3\xa9 A\xc3\xa9 a\xc3\x89"));
show('str_ireplace digits', str_ireplace('12', '_', '0123412'));
show('stripos high byte first', stripos("x\xffABC", "\xffabc"));
show('stripos at', stripos('aXbxcx', 'X', 2));
attempt('stripos bad off', fn() => stripos('abc', 'a', 4));
attempt('strripos bad off', fn() => strripos('abc', 'a', -4));

echo "-- explode limits\n";
show('plain', explode(',', 'a,b,,c'));
show('limit 2', explode(',', 'a,b,c', 2));
show('limit 1', explode(',', 'a,b,c', 1));
show('limit 0', explode(',', 'a,b,c', 0));
show('limit -1', explode(',', 'a,b,c', -1));
show('limit -3', explode(',', 'a,b,c', -3));
show('limit -5', explode(',', 'a,b,c', -5));
show('limit big', explode(',', 'a,b,c', 99));
show('multi-byte sep', explode('::', 'a::b:::c::', 3));
show('overlap sep', explode('aa', 'aaaaa'));
show('sep at ends', explode('|', '|a|'));
show('no sep', explode('|', 'abc', -1));
show('empty subject', explode(',', ''));
show('empty subject neg', explode(',', '', -1));
