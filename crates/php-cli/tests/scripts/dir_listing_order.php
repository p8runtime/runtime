<?php
// readdir() and the directory iterators list in the system's order (as
// scandir() with SCANDIR_SORT_NONE does); scandir() sorts by default.
$d = __DIR__ . '/dir_listing_order.tmp';
@mkdir($d);
foreach (['zeta', 'alpha', 'Mid', 'beta.txt', '0num'] as $n) {
    touch("$d/$n");
}
$raw = scandir($d, SCANDIR_SORT_NONE);
$h = opendir($d);
$read = [];
while (($e = readdir($h)) !== false) {
    $read[] = $e;
}
closedir($h);
$iter = [];
foreach (new DirectoryIterator($d) as $f) {
    $iter[] = $f->getFilename();
}
var_dump($read === $raw, $iter === $raw);
$sorted = $raw;
sort($sorted);
var_dump(scandir($d) === $sorted, scandir($d, SCANDIR_SORT_DESCENDING) === array_reverse($sorted));
var_dump(scandir($d));
foreach (glob("$d/*") as $p) {
    echo basename($p), "\n";
}
foreach (['zeta', 'alpha', 'Mid', 'beta.txt', '0num'] as $n) {
    unlink("$d/$n");
}
rmdir($d);
