<?php
// Changing an archive: rename, delete, comments, attributes, compression, undo.
function show($label, $v) {
    echo "== $label: ", is_array($v) ? json_encode($v) : var_export($v, true), "\n";
}
function entries($z) {
    $out = [];
    for ($i = 0; $i < $z->numFiles; $i++) {
        $s = $z->statIndex($i);
        $out[] = $s === false ? false : [$s['name'], $s['size'], $s['comp_method']];
    }
    return $out;
}
set_error_handler(function ($n, $s) { echo "[W: $s] "; return true; });

$base = sys_get_temp_dir() . '/p8zip_modify_' . getmypid();
mkdir($base);
chdir($base);

$z = new ZipArchive;
$z->open('m.zip', ZipArchive::CREATE);
$z->addFromString('a.txt', str_repeat('alpha ', 50));
$z->addFromString('b.txt', 'bravo');
$z->addFromString('c/d.txt', 'delta');
$z->close();

$z = new ZipArchive;
$z->open('m.zip');
show('entries', entries($z));
show('renameName', $z->renameName('b.txt', 'b2.txt'));
show('renameIndex', $z->renameIndex(2, 'c/e.txt'));
show('rename onto existing', $z->renameName('a.txt', 'b2.txt'));
show('status', $z->getStatusString());
show('rename missing', $z->renameName('zzz', 'y'));
show('rename bad index', $z->renameIndex(9, 'y'));
show('names', [$z->getNameIndex(0), $z->getNameIndex(1), $z->getNameIndex(2)]);
show('names unchanged', [$z->getNameIndex(1, ZipArchive::FL_UNCHANGED), $z->getNameIndex(2, ZipArchive::FL_UNCHANGED)]);
show('locate new', $z->locateName('b2.txt'));
show('locate old', $z->locateName('b.txt'));
show('locate old unchanged', $z->locateName('b.txt', ZipArchive::FL_UNCHANGED));
show('stat unchanged', $z->statIndex(1, ZipArchive::FL_UNCHANGED)['name']);

show('deleteName', $z->deleteName('a.txt'));
show('numFiles after delete', $z->numFiles);
show('getNameIndex deleted', $z->getNameIndex(0));
show('statIndex deleted', $z->statIndex(0));
show('locate deleted', $z->locateName('a.txt'));
show('delete twice', $z->deleteIndex(0));
show('unchangeIndex', $z->unchangeIndex(0));
show('restored', $z->getNameIndex(0));
show('unchangeName', $z->unchangeName('b.txt'));
show('unchangeAll', $z->unchangeAll());
show('after unchangeAll', entries($z));
show('close unchanged', $z->close());

$before = md5_file('m.zip');
$z->open('m.zip');
$z->renameIndex(0, 'tmp');
$z->setCommentIndex(0, 'tmp comment');
$z->unchangeIndex(0);
$z->close();
show('file untouched by an undone change', md5_file('m.zip') === $before);

// comments
$z->open('m.zip');
show('getCommentIndex', $z->getCommentIndex(0));
show('setCommentIndex', $z->setCommentIndex(0, 'first comment'));
show('setCommentName', $z->setCommentName('b.txt', 'second'));
show('setCommentName missing', $z->setCommentName('zzz', 'x'));
show('getCommentName', $z->getCommentName('b.txt'));
show('getCommentIndex unchanged', $z->getCommentIndex(0, ZipArchive::FL_UNCHANGED));
show('getArchiveComment', $z->getArchiveComment());
show('setArchiveComment', $z->setArchiveComment('the archive'));
show('comment property', $z->comment);
show('getArchiveComment unchanged', $z->getArchiveComment(ZipArchive::FL_UNCHANGED));
show('unchangeArchive', $z->unchangeArchive());
show('getArchiveComment', $z->getArchiveComment());
$z->setArchiveComment('kept');
show('close', $z->close());
$z->open('m.zip');
show('reopened comments', [$z->getCommentIndex(0), $z->getCommentName('b.txt'), $z->getArchiveComment(), $z->comment]);

// attributes, times and compression
show('getExternalAttributesIndex', $z->getExternalAttributesIndex(0, $os, $attr));
show('default os', $os);
show('setExternalAttributesIndex', $z->setExternalAttributesIndex(0, ZipArchive::OPSYS_UNIX, 0100755 << 16));
show('getExternalAttributesName', $z->getExternalAttributesName('a.txt', $os, $attr));
show('attrs', [$os, decoct($attr >> 16)]);
show('setMtimeName', $z->setMtimeName('a.txt', 86400 * 365 * 30));
show('mtime', $z->statName('a.txt')['mtime']);
show('setCompressionName store', $z->setCompressionName('a.txt', ZipArchive::CM_STORE));
show('setCompressionIndex deflate', $z->setCompressionIndex(1, ZipArchive::CM_DEFLATE));
show('status', $z->getStatusString());
show('close', $z->close());
$z->open('m.zip');
show('entries', entries($z));
show('attrs kept', (function ($z) { $z->getExternalAttributesName('a.txt', $os, $attr); return [$os, decoct($attr >> 16)]; })($z));
show('mtime kept', $z->statName('a.txt')['mtime']);
show('content kept', $z->getFromName('a.txt') === str_repeat('alpha ', 50));
show('isCompressionMethodSupported', [ZipArchive::isCompressionMethodSupported(ZipArchive::CM_STORE), ZipArchive::isCompressionMethodSupported(ZipArchive::CM_DEFLATE)]);

// replace and delete everything
show('addFromString replaces', $z->addFromString('b.txt', 'replaced'));
show('lastId', $z->lastId);
show('still two files', $z->numFiles);
show('replaced', $z->getFromName('b.txt'));
show('archive flag rdonly', [$z->getArchiveFlag(ZipArchive::AFL_RDONLY), $z->setArchiveFlag(ZipArchive::AFL_RDONLY, 1)]);
show('changed archive refuses rdonly', $z->status === ZipArchive::ER_CHANGED);
$z->unchangeAll();
show('rdonly now', [$z->setArchiveFlag(ZipArchive::AFL_RDONLY, 1), $z->getArchiveFlag(ZipArchive::AFL_RDONLY)]);
show('modify rdonly', $z->deleteIndex(0));
show('status', $z->getStatusString());
show('rdonly off', $z->setArchiveFlag(ZipArchive::AFL_RDONLY, 0));
$z->close();

$z->open('m.zip');
for ($i = $z->numFiles; $i;) $z->deleteIndex(--$i);
show('numFiles', $z->numFiles);
show('close empties', $z->close());
show('file removed', file_exists('m.zip'));
$z->open('m.zip', ZipArchive::CREATE);
$z->setArchiveFlag(ZipArchive::AFL_CREATE_OR_KEEP_FILE_FOR_EMPTY_ARCHIVE, 1);
show('close kept empty', [$z->close(), file_exists('m.zip')]);
$z->open('m.zip');
show('empty archive', [$z->numFiles, $z->getArchiveComment()]);
$z->close();

// properties
$z = new ZipArchive;
$z->open('p.zip', ZipArchive::CREATE);
$z->addFromString('x', 'y');
$z->setArchiveComment('pc');
$v = (array) $z;
show('props', [$v['lastId'], $v['status'], $v['numFiles'], basename($v['filename']), $v['comment']]);
show('isset', [isset($z->numFiles), isset($z->status), isset($z->nope), empty($z->status)]);
try { $z->numFiles = 5; } catch (Error $e) { show('write prop', $e->getMessage()); }
$z->close();
show('closed props', [$z->numFiles, $z->filename, $z->comment, $z->lastId]);
show('clone', (function ($z) { try { return clone $z; } catch (Error $e) { return $e->getMessage(); } })($z));
show('instanceof Countable', $z instanceof Countable);
unlink('p.zip');
@unlink('m.zip');
chdir('/');
rmdir($base);
