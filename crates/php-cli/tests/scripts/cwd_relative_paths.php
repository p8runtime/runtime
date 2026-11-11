<?php
// Relative paths after chdir(): every file and directory function, and
// include, resolve them against the working directory the script moved to
// (#459). Prints no path of this machine.
$base = sys_get_temp_dir() . '/p8-cwd-paths-' . getmypid();
mkdir($base);
chdir($base);
$real = getcwd();

echo "-- files\n";
var_dump(file_put_contents('a.txt', "one\ntwo\n"));
var_dump(file_get_contents('a.txt'));
var_dump(file('a.txt', FILE_IGNORE_NEW_LINES));
var_dump(is_file('a.txt'), is_dir('a.txt'), file_exists('a.txt'), filesize('a.txt'));
var_dump(is_readable('a.txt'), is_writable('a.txt'));
$f = fopen('a.txt', 'a');
fwrite($f, "three\n");
fclose($f);
$f = fopen('./a.txt', 'r');
var_dump(fgets($f));
fclose($f);
var_dump(copy('a.txt', 'b.txt'));
var_dump(rename('b.txt', 'c.txt'));
var_dump(file_exists('b.txt'), file_get_contents('c.txt') === file_get_contents('a.txt'));
var_dump(touch('t.txt'), is_file('t.txt'));
var_dump(realpath('a.txt') === "$real/a.txt");
var_dump(realpath('missing.txt'));
var_dump(realpath('.') === $real);

echo "-- directories\n";
var_dump(mkdir('d'));
var_dump(mkdir('d/e/f', 0777, true));
var_dump(is_dir('d/e'), is_dir('./d/e/f'));
file_put_contents('d/x.php', '<?php echo "included d/x.php\n"; return 7;');
file_put_contents('d/e/y.txt', 'y');
var_dump(scandir('d'));
$h = opendir('d');
$names = [];
while (($n = readdir($h)) !== false) {
    $names[] = $n;
}
closedir($h);
sort($names);
var_dump($names);
$dir = dir('d/e');
$names = [];
while (($n = $dir->read()) !== false) {
    $names[] = $n;
}
$dir->close();
sort($names);
var_dump($names);
var_dump(glob('*.txt'));
var_dump(glob('d/*'));
var_dump(glob('d/e/*.txt'));

echo "-- include\n";
var_dump(include 'd/x.php');
var_dump(include './d/x.php');
chdir('d');
var_dump(require 'x.php');
var_dump(include_once './x.php');
var_dump(include_once 'x.php');
var_dump(@include 'a.txt.php');
chdir('..');

echo "-- removing\n";
var_dump(unlink('d/x.php'), unlink('d/e/y.txt'));
var_dump(rmdir('d/e/f'), rmdir('d/e'), rmdir('d'));
var_dump(unlink('a.txt'), unlink('c.txt'), unlink('t.txt'));
var_dump(scandir('.'));
chdir(sys_get_temp_dir());
var_dump(rmdir($base));
