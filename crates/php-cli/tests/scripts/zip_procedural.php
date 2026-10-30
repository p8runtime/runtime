<?php
// The deprecated zip_* functions, over an archive ZipArchive wrote.
function show($label, $v) {
    echo "== $label: ", is_array($v) ? json_encode($v) : var_export($v, true), "\n";
}
set_error_handler(function ($n, $s) { echo "[", $n === E_DEPRECATED ? "D" : "W", ": $s] "; return true; });

$base = sys_get_temp_dir() . '/p8zip_proc_' . getmypid();
mkdir($base);
chdir($base);

$z = new ZipArchive;
$z->open('p.zip', ZipArchive::CREATE);
$z->addFromString('one.txt', str_repeat('one ', 100));
$z->addFromString('two.txt', 'two');
$z->addEmptyDir('dir');
$z->setCompressionName('two.txt', ZipArchive::CM_STORE);
$z->close();

$zip = zip_open('p.zip');
show('zip_open', get_resource_type($zip));
$rows = [];
while ($entry = zip_read($zip)) {
    $rows[] = [get_resource_type($entry), zip_entry_name($entry), zip_entry_filesize($entry), zip_entry_compressionmethod($entry), zip_entry_open($zip, $entry, 'rb')];
    $got = '';
    while (($chunk = zip_entry_read($entry, 64)) !== '') $got .= $chunk;
    $rows[] = strlen($got);
    show('close entry', zip_entry_close($entry));
}
show('rows', $rows);
show('zip_read at end', zip_read($zip));
zip_close($zip);

$zip = zip_open('p.zip');
$first = zip_read($zip);
show('default read length', strlen(zip_entry_read($first)));
show('read all', strlen(zip_entry_read($first, 1000)));
show('then empty', zip_entry_read($first, 10));
show('compressed size is an int', is_int(zip_entry_compressedsize($first)));
zip_close($zip);

show('zip_open missing', zip_open('missing.zip'));
file_put_contents('junk.zip', str_repeat('junk', 20));
show('zip_open junk', zip_open('junk.zip'));
try { zip_open(''); } catch (ValueError $e) { show('empty name', $e->getMessage()); }
try { zip_close('x'); } catch (TypeError $e) { show('bad arg', $e->getMessage()); }
try { zip_entry_name($zip); } catch (TypeError $e) { show('closed', $e->getMessage()); }

unlink('p.zip');
unlink('junk.zip');
chdir('/');
rmdir($base);
