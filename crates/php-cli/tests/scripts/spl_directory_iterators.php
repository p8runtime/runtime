<?php
// SplFileInfo and the directory iterators over a small tree; listings are
// sorted (directory order is the OS's) and paths shown relative to BASE.
$base = sys_get_temp_dir() . '/p8-spl-dir-' . getmypid();
@mkdir("$base/sub/deeper", 0777, true);
file_put_contents("$base/a.txt", 'aaa');
file_put_contents("$base/b.html", '<b>');
file_put_contents("$base/sub/c.html", '<c>');
file_put_contents("$base/sub/deeper/d.HTML", '<d>');
symlink("$base/a.txt", "$base/link");
$real = realpath($base);
$rel = fn($p) => str_replace([$real, $base], 'BASE', (string) $p);
function show($label, $list) { sort($list); echo $label, ': ', implode(', ', $list), "\n"; }

$i = new SplFileInfo("$base/a.txt");
echo $rel($i->getPath()), ' ', $i->getFilename(), ' ', $i->getExtension(), ' ', $i->getBasename('.txt'), ' ', $rel($i->getPathname()), ' ', $rel($i), "\n";
var_dump($i->getSize(), $i->getType(), $i->isFile(), $i->isDir(), $i->isLink(), $i->isReadable(), $i->isWritable(), $i->isExecutable(), decoct($i->getPerms() & 0777) !== '', is_int($i->getMTime()), $rel($i->getRealPath()));
$l = new SplFileInfo("$base/link");
var_dump($l->getType(), $l->isLink(), $l->isFile(), $rel($l->getLinkTarget()), $rel($l->getRealPath()));
foreach (['', '/', 'x', 'dir/', 'a/b/', '/a/b.c.d', 'a//b', '.hidden', 'noext.'] as $p) {
    $f = new SplFileInfo($p);
    echo json_encode([$p, $f->getPath(), $f->getFilename(), $f->getExtension(), $f->getBasename(), $f->getPathname()]), "\n";
}
$m = new SplFileInfo("$base/missing");
var_dump($m->isFile(), $m->isReadable(), $m->getRealPath());
foreach (['getSize', 'getMTime', 'getType', 'getPerms', 'getOwner'] as $f) {
    try { $m->$f(); } catch (RuntimeException $e) { echo get_class($e), ': ', $rel($e->getMessage()), "\n"; }
}
echo $rel($i->getPathInfo()), ' ', get_class($i->getFileInfo()), ' ', $rel($i->getFileInfo()), "\n";
var_dump(str_replace($base, 'BASE', print_r($i, true)));

$names = [];
foreach (new DirectoryIterator($base) as $k => $f) {
    $names[] = $f->getFilename() . ($f->isDot() ? '(dot)' : '') . ($f->isDir() ? '/' : '') . '=' . $rel($f->getPathname()) . ':' . (string) $f . ':' . $f->getExtension();
    if (!is_int($k)) echo "key not int\n";
}
show('DirectoryIterator', $names);
$d = new DirectoryIterator($base);
var_dump($d->key(), $d->valid(), $d->current() === $d);
$d->seek(3); var_dump($d->key());
try { $d->seek(99); } catch (OutOfBoundsException $e) { echo $e->getMessage(), "\n"; }

$names = [];
foreach (new FilesystemIterator($base) as $k => $f) $names[] = $rel($k) . '=>' . get_class($f) . ':' . $f->getFilename();
show('FilesystemIterator', $names);
$names = [];
foreach (new FilesystemIterator($base, FilesystemIterator::KEY_AS_FILENAME | FilesystemIterator::CURRENT_AS_PATHNAME) as $k => $f) $names[] = "$k=>" . $rel($f);
show('KEY_AS_FILENAME|CURRENT_AS_PATHNAME', $names);
$names = [];
foreach (new FilesystemIterator($base, FilesystemIterator::CURRENT_AS_SELF) as $k => $f) $names[] = $f->getFilename();
show('CURRENT_AS_SELF', $names);
$fi = new FilesystemIterator($base);
var_dump($fi->getFlags(), FilesystemIterator::SKIP_DOTS);
$fi->setFlags(FilesystemIterator::UNIX_PATHS);
var_dump($fi->getFlags());

$names = [];
foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator($base, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::SELF_FIRST) as $p => $f) $names[] = $rel($p) . ($f->isDir() ? '/' : '');
show('Recursive SELF_FIRST', $names);
$names = [];
$it = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($base));
foreach ($it as $p => $f) $names[] = $rel($p) . '|' . $it->getSubPath() . '|' . $it->getSubPathname();
show('Recursive leaves (with dots)', $names);
// WordPress's template lookup.
$html = [];
foreach (new RegexIterator(new RecursiveIteratorIterator(new RecursiveDirectoryIterator($base)), '/^.+\.html$/i', RecursiveRegexIterator::GET_MATCH) as $path => $m) $html[] = $rel($path) . ' ' . json_encode(array_map($rel, $m));
show('RegexIterator GET_MATCH', $html);
$r = new RecursiveDirectoryIterator($base);
foreach ($r as $f) { if ($r->hasChildren()) { $c = $r->getChildren(); echo get_class($c), ' sub=', $c->getSubPath(), "\n"; break; } }

$names = [];
$g = new GlobIterator("$base/*.*");
foreach ($g as $k => $f) $names[] = $rel($k) . '=>' . $f->getFilename();
show('GlobIterator', $names);
var_dump(count($g), $g->count());

foreach (['', "$base/nope"] as $bad) {
    try { new DirectoryIterator($bad); } catch (Throwable $e) { echo get_class($e), ': ', $rel($e->getMessage()), "\n"; }
}
try { (new DirectoryIterator($base))->__construct($base); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
class Lazy extends DirectoryIterator { function __construct() {} }
try { (new Lazy)->valid(); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }

unlink("$base/link"); unlink("$base/sub/deeper/d.HTML"); unlink("$base/sub/c.html"); unlink("$base/b.html"); unlink("$base/a.txt"); rmdir("$base/sub/deeper"); rmdir("$base/sub"); rmdir($base);
