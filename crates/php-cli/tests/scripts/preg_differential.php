<?php
$cases = [
 ['/(\d+)-(\d+)/', 'a 12-34 b 5-6'], ['/\b\w+\b/', 'Hello, world! foo_bar 42'], ['/^\s*$/m', "a\n\n  \nb"],
 ['/(?<year>\d{4})-(?<m>\d\d)/', 'on 2024-06 and 1999-12'], ['/a|ab|abc/', 'abc'], ['/(a|ab)(c|bcd)(d*)/', 'abcd'],
 ['/x*/', 'axxb'], ['/(?:)/', 'abc'], ['/./u', 'héllo wörld'], ['/[[:punct:]]+/', 'a!?b..c'], ['/\p{Lu}\p{Ll}+/u', 'Été Ölm Ab'],
 ['/(a)?b/', 'b'], ['/(a)?(b)?c/', 'c'], ['/(?i)straße/u', 'STRASSE straße STRAßE'], ['/\d+(?=px)/', '12px 30em 7px'],
 ['/(?<!\$)\b\d+/', '$100 200'], ['/(\w)\1/', 'hello  bookkeeper'], ['/<([a-z]+)>.*?<\/\1>/s', "<b>x</b><i>\ny</i>"],
 ['/\G\d/', '123a45'], ['/^/m', "a\nb\nc"], ['/$/m', "a\nb\n"], ['/a$/', "a\n"], ['/a\Z/', "a\n"], ['/a\z/', "a\n"],
 ['/[^\x00-\x7F]+/', "abc\xC3\xA9d"], ['/(?|(a)|(b))\1/', 'aa bb ab'], ['/(?>a+)b/', 'aaab'], ['/\R/', "a\r\nb\nc\rd"],
 ['/\h+/', "a \t b"], ['/(?x) a  b # comment
   c/', 'abc'], ['/\Qa.b\E+/', 'a.bbb'], ['/[a-z-]+/', 'foo-bar baz'], ['/[\w-]+@[\w.]+/', 'x: john-d@ex.com.'],
 ['/(\d+)(?:px|em)?/', '10px 5 3em'], ['/^(?:(\d+)|(\w+))$/', 'abc'], ['/(a+)+$/', 'aaaa'], ['/\bé/u', 'café é'],
 ['/[[:alpha:]]+/u', 'naïve café'], ['/\w+/u', 'naïve café'], ['/\w+/', 'naïve'], ['/(?<=\d{3})x/', '123x 12x'],
 ['/(?(?=\d)\d+|[a-z]+)/', '12ab'], ['/(\()?\d+(?(1)\))/', '(12) 34 (5'], ['/a(*SKIP)(*FAIL)|b/', 'ab'],
 ['/(?:a(*THEN)b|ac)/', 'ac'], ['/\X/u', "e\u{301}\u{1F468}\u{200D}\u{1F469}x"], ['/[\p{Han}]+/u', '漢字かな'],
 ['/(?i)[a-c]+/', 'ABCabcD'], ['/(?i)[^a-c]+/', 'ABCxyz'], ['/\d{2,}?/', '12345'], ['/(?U)a+/', 'aaa'], ['/a{,3}/', 'aaaa'],
 ['/(?<n>a)|(?<n>b)/J', 'b'], ['/(a)(?:x|(b))*/', 'axbxb'], ['/(?:(a)|b)*/', 'ab'], ['/^(\w+)\s(?1)$/', 'foo bar'],
 ['/\((?:[^()]++|(?R))*\)/', '(a(b)c) (d'], ['/(?s).+/', "a\nb"], ['/.+/', "a\nb"], ['/[\d\s]+/', '1 2 3x'],
 ['/\S+/', ' ab  cd '], ['/\p{N}+/u', 'x٣٤5'], ['/[^\p{L}]+/u', 'ab12ü!'], ['/(?m)^\w/', "ab\ncd\n"],
 ['/\Bo\B/', 'foo boo o'], ['/(?<word>\w+)\s+\k<word>/', 'the the cat'], ['/(?P<x>\d)(?P=x)/', '1123'],
 ['/(*UTF)\x{e9}/', 'café'], ['/\x41\101\o{102}/', 'AAB'], ['/\cA/', "\x01"], ['/[\\\\]+/', 'a\\\\b'], ['/\//', 'a/b'],
];
foreach ($cases as [$p, $s]) {
  $r = @preg_match_all($p, $s, $m, PREG_OFFSET_CAPTURE);
  echo $p, ' => ', json_encode([$r, $m, preg_last_error()]), "\n";
  echo '  split ', json_encode(@preg_split($p, $s, -1, PREG_SPLIT_DELIM_CAPTURE)), "\n";
  echo '  repl ', json_encode(@preg_replace($p, '[$0|\1]', $s)), "\n";
}
echo json_encode(preg_replace_callback('/\d+/', fn($m) => $m[0]*2, 'a1 b22 c333')), "\n";
echo json_encode(preg_split('//', 'abc', -1, PREG_SPLIT_NO_EMPTY)), json_encode(preg_split('/(-)/', 'a-b-c', 2, PREG_SPLIT_DELIM_CAPTURE)), "\n";
echo json_encode(preg_quote('a.b*c?d+e(f)g[h]i{j}k=l!m<n>o|p:q-r#s/t', '/')), "\n";
echo json_encode(preg_grep('/^\d+$/', ['1', 'a', 'x'=>'22', '3b'])), json_encode(preg_grep('/^\d+$/', ['1', 'a'], PREG_GREP_INVERT)), "\n";
var_dump(preg_match('/(a)(b)?(c)?/', 'a', $m, PREG_UNMATCHED_AS_NULL), $m);
var_dump(preg_match_all('/(a)(b)?/', 'aab', $m, PREG_SET_ORDER), $m);
var_dump(preg_replace(['/a/', '/b/'], ['b', 'c'], 'ab'), preg_replace('/x/', 'y', ['k'=>'xx', 'z']), preg_filter('/x/', 'y', ['xa', 'b']));
var_dump(preg_match('/(*MARK:A)a|(*MARK:B)b/', 'b', $m), $m);
var_dump(preg_replace('/(\w+) (\w+)/', '${2}1 \\2 $1 \\\\1 \$1', 'hello world'));
