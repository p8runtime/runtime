<?php
// addGlob() and addPattern(): which files are added, under which names.
function show($label, $v) {
    echo "== $label: ", is_array($v) ? json_encode($v) : var_export($v, true), "\n";
}
function names($z) {
    $n = [];
    for ($i = 0; $i < $z->numFiles; $i++) $n[] = $z->getNameIndex($i);
    return $n;
}
set_error_handler(function ($n, $s) { echo "[W: $s] "; return true; });

$base = sys_get_temp_dir() . '/p8zip_glob_' . getmypid();
mkdir($base);
chdir($base);
mkdir('src');
mkdir('src/sub');
file_put_contents('src/a.txt', 'a');
file_put_contents('src/b.txt', 'b');
file_put_contents('src/c.md', 'c');
file_put_contents('src/sub/d.txt', 'd');

$z = new ZipArchive;
$z->open('g.zip', ZipArchive::CREATE);
show('addGlob', $z->addGlob('src/*.txt'));
show('names', names($z));
show('addGlob remove_all_path + add_path', $z->addGlob('src/*.md', 0, ['remove_all_path' => true, 'add_path' => 'flat/']));
show('addGlob remove_path', $z->addGlob('src/sub/*', 0, ['remove_path' => 'src']));
show('addGlob brace', $z->addGlob('src/{a,c}.*', GLOB_BRACE, ['add_path' => 'brace/', 'remove_path' => 'src']));
show('names', names($z));
show('addGlob again fails without overwrite', $z->addGlob('src/*.txt', 0, ['flags' => 0]));
show('status', $z->getStatusString());
show('addGlob again overwrites', $z->addGlob('src/*.txt', 0, ['flags' => ZipArchive::FL_OVERWRITE]));
show('count', $z->numFiles);
show('addGlob onlydir', $z->addGlob('src/*', GLOB_ONLYDIR, ['add_path' => 'dirs/']));
show('a directory is no source to write', $z->deleteName('dirs/src/sub'));
show('addGlob compression', $z->addGlob('src/c.md', 0, ['remove_path' => 'src', 'add_path' => 'stored/', 'comp_method' => ZipArchive::CM_STORE]));
show('stored method', $z->statName('stored/c.md')['comp_method']);
set_error_handler(fn() => true);
show('too long add_path', $z->addGlob('src/c.md', 0, ['add_path' => str_repeat('A', PHP_MAXPATHLEN - 2)]));
restore_error_handler();
show('close', $z->close());

$z = new ZipArchive;
$z->open('g.zip');
show('final', names($z));
show('content', $z->getFromName('sub/d.txt'));
$z->close();

$z = new ZipArchive;
$z->open('p.zip', ZipArchive::CREATE);
show('addPattern', $z->addPattern('/\.txt$/', 'src'));
show('addPattern remove_all_path', $z->addPattern('/^c\./', 'src', ['remove_all_path' => true, 'add_path' => 'md/']));
show('addPattern remove_path', $z->addPattern('/./', 'src/sub', ['remove_path' => 'src/']));
show('addPattern nothing', $z->addPattern('/\.nope$/', 'src'));
show('addPattern bad regex', $z->addPattern('/oops', 'src'));
show('names', names($z));
$z->close();

$rm = function ($d) use (&$rm) {
    foreach (array_diff(scandir($d), ['.', '..']) as $e) { is_dir("$d/$e") ? $rm("$d/$e") : unlink("$d/$e"); }
    rmdir($d);
};
$rm('src');
unlink('g.zip');
unlink('p.zip');
chdir('/');
rmdir($base);
