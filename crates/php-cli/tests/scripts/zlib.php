<?php
$d = str_repeat("hello world ", 50) . implode(',', range(1, 200));
foreach ([-1, 0, 1, 6, 9] as $l) {
    var_dump(gzuncompress(gzcompress($d, $l)) === $d, gzinflate(gzdeflate($d, $l)) === $d, gzdecode(gzencode($d, $l)) === $d);
    echo bin2hex(substr(gzencode($d, $l), 0, 10)), ' ', bin2hex(substr(gzcompress($d, $l), 0, 2)), "\n";
}
var_dump(zlib_decode(gzencode('x')), zlib_decode(gzcompress('y')), zlib_decode(gzdeflate('z')));
var_dump(gzcompress(''), strlen(gzencode('')), bin2hex(gzdeflate('')));
var_dump(@gzuncompress('garbage'), @gzinflate(''), gzinflate(gzdeflate(str_repeat('a', 100)), 10));
try { gzcompress('x', 10); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { zlib_encode('x', 99); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$f = sys_get_temp_dir() . '/p8zlib.gz';
$w = gzopen($f, 'w9'); var_dump(gzwrite($w, "line1\nline2\n"), gzputs($w, "line3"), gztell($w), gzread($w, 1), gzrewind($w)); gzclose($w);
$r = gzopen($f, 'rb'); var_dump(gzgets($r), gzgetc($r), gzread($r, 4), gzeof($r), gzread($r, 100), gzeof($r), gzwrite($r, 'x'), gzseek($r, 2), gztell($r), @gzseek($r, 0, SEEK_END)); gzclose($r);
var_dump(gzfile($f), file_get_contents("compress.zlib://$f"), readgzfile($f));
var_dump(@gzopen($f, 'r+'), @gzopen($f, 'e'));
unlink($f);
$c = deflate_init(ZLIB_ENCODING_GZIP, ['level' => 9]); $z = deflate_add($c, "abc", ZLIB_NO_FLUSH) . deflate_add($c, "def", ZLIB_FINISH);
var_dump(gzdecode($z), get_class($c));
$i = inflate_init(ZLIB_ENCODING_DEFLATE); var_dump(inflate_add($i, gzcompress("streamed")), inflate_get_status($i));
try { new DeflateContext(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(extension_loaded('zlib'), ZLIB_ENCODING_RAW, zlib_get_coding_type());
