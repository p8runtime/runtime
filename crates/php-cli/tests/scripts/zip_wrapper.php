<?php
// The zip:// stream wrapper reads one entry of an archive.
function show($label, $v) {
    echo "== $label: ", is_array($v) ? json_encode($v) : var_export($v, true), "\n";
}
set_error_handler(function ($n, $s) { echo "[W: ", preg_replace('~\(zip://[^)]*\)~', '(ZIPURL)', $s), "] "; return true; });

$base = sys_get_temp_dir() . '/p8zip_wrap_' . getmypid();
mkdir($base);
chdir($base);
$z = new ZipArchive;
$z->open('w.zip', ZipArchive::CREATE);
$z->addFromString('text.txt', "first line\nsecond line\nthird line\n");
$z->addFromString('dir/inner.bin', "\0\1\2\3");
$z->close();
$archive = $base . '/w.zip';

$h = fopen("zip://$archive#text.txt", 'r');
show('fopen', is_resource($h));
show('fgets', fgets($h));
show('fgets', fgets($h));
show('rest', stream_get_contents($h));
show('eof', feof($h));
fclose($h);
show('file_get_contents', file_get_contents("zip://$archive#dir/inner.bin"));
show('file()', file("zip://$archive#text.txt", FILE_IGNORE_NEW_LINES));
show('missing entry', fopen("zip://$archive#nope", 'r'));
show('missing archive', fopen("zip://$base/none.zip#text.txt", 'r'));
show('no entry given', fopen("zip://$archive", 'r'));
show('write mode', fopen("zip://$archive#text.txt", 'w'));
show('copy', copy("zip://$archive#text.txt", 'copied.txt') && file_get_contents('copied.txt') === "first line\nsecond line\nthird line\n");

// getStream gives the same bytes
$z->open('w.zip');
$h = $z->getStream('text.txt');
show('getStream', stream_get_contents($h));
fclose($h);
$h = $z->getStreamIndex(1);
show('getStreamIndex', bin2hex(stream_get_contents($h)));
fclose($h);
$z->close();

unlink('w.zip');
unlink('copied.txt');
chdir('/');
rmdir($base);
