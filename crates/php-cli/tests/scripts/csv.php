<?php
$cases = ['a,b,c', '"a ""q"" b",c', ' "sp" ,x', '"multi
line",z', 'a,', '', '"esc \" q",n', '"unterminated', 'x,"y"tail,z', "crlf,end\r\n", '  plain  , t ', '"a"'];
foreach ($cases as $c) var_dump(str_getcsv($c, ',', '"', '\\'));
var_dump(str_getcsv('a;b|c', ';', '|', ''));
var_dump(str_getcsv('a,b'));
try { str_getcsv('a', ',,'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { str_getcsv('a', ',', '"', 'xx'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$f = fopen('php://memory', 'w+');
var_dump(fputcsv($f, ['plain', 'with space', 'q"uote', "nl\nx", 'es\\"c', 12, 1.5, null, true], ',', '"', '\\'));
fputcsv($f, ['x', 'y'], ';', "'", '', "\r\n");
fwrite($f, "\"open\nnext\",after\n\nlast,row");
rewind($f);
echo stream_get_contents($f), "\n";
rewind($f);
while (($row = fgetcsv($f, null, ',', '"', '\\')) !== false) var_dump($row);
rewind($f);
var_dump(fgetcsv($f, 5, ',', '"', '\\'));
