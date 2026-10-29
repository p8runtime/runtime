<?php
// mb_split / mb_ereg / mb_ereg_replace(_callback) / mb_ereg_match: Oniguruma's
// options and group rules (named groups make plain groups non-capturing).
var_dump(mb_split('\)\s+AND\s+\(', "a = 1) AND (b = 2"));
var_dump(mb_split('\s*,\s*', 'α, β ,γ,,δ'), mb_split(',', 'a,b,c,d', 2), mb_split(',', 'a,b', 1), mb_split('', 'abc'), mb_split('x*', 'axbc'));
var_dump(mb_ereg('(\d+)-(?<word>\w+)(z)?', 'id 42-größe!', $m), $m);
var_dump(mb_ereg('nope', 'abc', $m2), $m2);
var_dump(mb_eregi('ÄB', 'xäb'), mb_ereg_match('a.c', "a\nc"), mb_ereg_match('b', 'abc'), mb_ereg_match('a', 'abc'));
var_dump(mb_ereg_replace('(\w)(\d)', '\2\1[\k<x>]', 'a1 b2 c'), mb_ereg_replace('(?<x>o)', '<\k<x>\k\'x\'>', 'foo'), mb_eregi_replace('É', 'e', 'Été'));
var_dump(mb_ereg_replace('', '-', 'ab'), mb_ereg_replace('x*', '-', 'axb'));
var_dump(mb_ereg_replace_callback('(\d+)', fn($m) => $m[1] * 2, 'a1 b22 c'));
var_dump(mb_ereg_replace('^b', 'X', "a\nb"), mb_ereg_replace('^b', 'X', "a\nb", 'm'), mb_ereg_replace('a.b', 'X', "a\nb"), mb_ereg_replace('a.b', 'X', "a\nb", 'x'));
var_dump(mb_regex_set_options(), mb_split('^', "a\nb"));
