<?php
// What WordPress uses ZipArchive for: build an archive, read it back, extract it.
function show($label, $v) {
    echo "== $label: ", is_array($v) ? json_encode($v) : var_export($v, true), "\n";
}
function stat_of($z, $i) {
    $s = $z->statIndex($i);
    if ($s === false) return false;
    unset($s['mtime'], $s['comp_size']);
    return $s;
}
set_error_handler(function ($n, $s) { echo "[W: $s] "; return true; });

$base = sys_get_temp_dir() . '/p8zip_basics_' . getmypid();
mkdir($base);
chdir($base);
file_put_contents('on-disk.txt', "line one\nline two\n");
file_put_contents('slice.bin', '0123456789abcdefghij');

$z = new ZipArchive;
show('numFiles before open', $z->numFiles);
show('open CREATE', $z->open('out.zip', ZipArchive::CREATE));
show('addFromString', $z->addFromString('hello.txt', "hello world\nhello world\nhello world\n"));
show('addFromString dir', $z->addFromString('dir/nested.txt', 'nested'));
show('addEmptyDir', $z->addEmptyDir('empty'));
show('addEmptyDir slash', $z->addEmptyDir('empty2/'));
show('addFile', $z->addFile('on-disk.txt', 'copied.txt'));
show('addFile slice', $z->addFile('slice.bin', 'slice.txt', 5, 6));
show('addFile default name', $z->addFile('on-disk.txt'));
show('addFile missing', $z->addFile('no-such-file'));
show('add duplicate', $z->addFromString('hello.txt', 'again', 0));
show('status after dup', $z->getStatusString());
show('overwrite', $z->addFromString('hello.txt', "hello world\nhello world\nhello world\n", ZipArchive::FL_OVERWRITE));
show('lastId', $z->lastId);
show('numFiles', $z->numFiles);
show('count', count($z));
show('close', $z->close());
show('file exists', file_exists('out.zip'));

$z = new ZipArchive;
show('open RDONLY', $z->open('out.zip', ZipArchive::RDONLY));
show('numFiles', $z->numFiles);
for ($i = 0; $i < $z->numFiles; $i++) {
    show("name $i", $z->getNameIndex($i));
}
show('stat 0', stat_of($z, 0));
show('stat dir', stat_of($z, 2));
show('stat missing', $z->statIndex(99));
show('status', $z->getStatusString());
show('statName', $z->statName('copied.txt')['size']);
show('locateName', $z->locateName('hello.txt'));
show('locateName nocase', $z->locateName('HELLO.TXT', ZipArchive::FL_NOCASE));
show('locateName nodir', $z->locateName('nested.txt', ZipArchive::FL_NODIR));
show('locateName miss', $z->locateName('nope'));
show('getFromName', $z->getFromName('hello.txt'));
show('getFromName len', $z->getFromName('hello.txt', 5));
show('getFromIndex', $z->getFromIndex(1));
show('getFromName copied', $z->getFromName('copied.txt'));
show('getFromName slice', $z->getFromName('slice.txt'));
show('getFromName miss', $z->getFromName('nope'));
show('status', $z->getStatusString());
show('getFromIndex miss', $z->getFromIndex(99));
$h = $z->getStream('hello.txt');
show('getStream', is_resource($h));
show('fgets', fgets($h));
show('fread', fread($h, 5));
show('rest', stream_get_contents($h));
show('eof', feof($h));
fclose($h);
show('getStream miss', $z->getStream('nope'));
show('attrs', (function ($z) { $z->getExternalAttributesIndex(0, $os, $attr); return [$os, $attr >> 16 & 0170000]; })($z));
show('modify refused', $z->addFromString('x', 'y'));
show('status', $z->getStatusString());
show('close', $z->close());

mkdir('extracted');
$z = new ZipArchive;
$z->open('out.zip');
show('extractTo', $z->extractTo('extracted'));
$found = [];
$it = new RecursiveIteratorIterator(new RecursiveDirectoryIterator('extracted', FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::SELF_FIRST);
foreach ($it as $f) { $found[] = substr($f->getPathname(), 10) . ($f->isDir() ? '/' : ':' . $f->getSize()); }
sort($found);
show('extracted', $found);
show('extractTo one', $z->extractTo('only', 'dir/nested.txt'));
show('only', file_get_contents('only/dir/nested.txt'));
show('extractTo list', $z->extractTo('some', ['hello.txt', 'copied.txt']));
show('some', array_values(array_diff(scandir('some'), ['.', '..'])));
show('extractTo missing', $z->extractTo('missing', 'nope'));
show('extractTo empty array', $z->extractTo('missing', []));
show('close', $z->close());

// open errors and flags
$z = new ZipArchive;
show('open missing', $z->open('missing.zip'));
show('open missing status', $z->getStatusString());
file_put_contents('junk.zip', str_repeat('not a zip ', 10));
show('open junk', $z->open('junk.zip'));
show('open EXCL existing', $z->open('out.zip', ZipArchive::EXCL | ZipArchive::CREATE));
show('open CHECKCONS', $z->open('out.zip', ZipArchive::CHECKCONS));
show('numFiles', $z->numFiles);
show('OVERWRITE', $z->open('out.zip', ZipArchive::OVERWRITE));
show('numFiles', $z->numFiles);
$z->addFromString('fresh.txt', 'fresh');
show('close', $z->close());
$z->open('out.zip');
show('names after overwrite', [$z->getNameIndex(0), $z->numFiles]);
$z->close();
show('constants', [ZipArchive::ER_NOENT, ZipArchive::ER_EXISTS, ZipArchive::ER_NOZIP, ZipArchive::ER_INCONS, ZipArchive::FL_OVERWRITE, ZipArchive::CM_DEFLATE]);
show('extension', [extension_loaded('zip'), class_exists('ZipArchive'), function_exists('zip_open')]);

// the archive is written when the object is let go without close()
$z = new ZipArchive;
$z->open('implicit.zip', ZipArchive::CREATE);
$z->addFromString('a', 'b');
unset($z);
show('implicit close wrote', file_exists('implicit.zip'));

foreach (['on-disk.txt', 'slice.bin', 'out.zip', 'junk.zip', 'implicit.zip'] as $f) unlink($f);
$rm = function ($d) use (&$rm) {
    foreach (array_diff(scandir($d), ['.', '..']) as $e) { is_dir("$d/$e") ? $rm("$d/$e") : unlink("$d/$e"); }
    rmdir($d);
};
foreach (['extracted', 'only', 'some', 'missing'] as $d) if (is_dir($d)) $rm($d);
chdir('/');
rmdir($base);
