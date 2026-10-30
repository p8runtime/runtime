<?php
// Arguments, errors and the states ZipArchive reports them in.
function t($label, $f) {
    echo "== $label: ";
    try { $r = $f(); echo is_array($r) ? json_encode($r) : var_export($r, true); }
    catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(); }
    echo "\n";
}
set_error_handler(function ($n, $s) { echo "[W: $s] "; return true; });

$base = sys_get_temp_dir() . '/p8zip_errors_' . getmypid();
mkdir($base);
chdir($base);
file_put_contents('data.txt', str_repeat('0123456789', 20));

$z = new ZipArchive;
t('count unopened', fn() => $z->count());
t('addFromString unopened', fn() => $z->addFromString('a', 'b'));
t('statIndex unopened', fn() => $z->statIndex(0));
t('close unopened', fn() => $z->close());
t('getStatusString unopened', fn() => $z->getStatusString());
t('open empty name', fn() => $z->open(''));
t('open nul', fn() => $z->open("a\0b"));
t('open missing', fn() => $z->open('nonexistent.zip'));
t('open type', fn() => $z->open([]));
t('open too many', fn() => $z->open('a', 0, 0));
t('status unopened', fn() => [$z->status, $z->statusSys, $z->numFiles, $z->filename, $z->comment, $z->lastId]);

$z->open('e.zip', ZipArchive::CREATE);
t('addFromString', fn() => $z->addFromString('a', 'x'));
t('addFromString empty name', fn() => $z->addFromString('', 'x'));
t('addEmptyDir empty', fn() => $z->addEmptyDir(''));
t('addEmptyDir twice', fn() => [$z->addEmptyDir('d'), $z->addEmptyDir('d'), $z->lastId, $z->getStatusString()]);
t('addEmptyDir overwrite', fn() => [$z->addEmptyDir('d', ZipArchive::FL_OVERWRITE), $z->lastId]);
t('addFile empty', fn() => $z->addFile(''));
t('addFile missing', fn() => $z->addFile('/nonexistent/zzz'));
t('addFile nul', fn() => $z->addFile("a\0b"));
t('addFile bad length', fn() => $z->addFile('data.txt', 'n1', 0, -5));
t('addFile bad start', fn() => $z->addFile('data.txt', 'n2', -1));
t('addFile start past end', fn() => $z->addFile('data.txt', 'n3', 300));
t('addFile length past end', fn() => $z->addFile('data.txt', 'n4', 0, 1000));
t('addFile unchecked length', fn() => $z->addFile('data.txt', 'n5', 0, ZipArchive::LENGTH_UNCHECKED));
t('addFile OPEN_FILE_NOW', fn() => [$z->addFile('data.txt', 'n6', 10, 5, ZipArchive::FL_OPEN_FILE_NOW), $z->lastId]);
t('replaceFile bad index', fn() => [$z->replaceFile('data.txt', 99), $z->getStatusString()]);
t('replaceFile negative', fn() => $z->replaceFile('data.txt', -1));
t('replaceFile', fn() => [$z->replaceFile('data.txt', 0, 2, 4), $z->statIndex(0)['size']]);
t('renameIndex empty', fn() => $z->renameIndex(0, ''));
t('renameIndex negative', fn() => $z->renameIndex(-1, 'x'));
t('renameName empty', fn() => $z->renameName('', 'b'));
t('renameName empty new', fn() => $z->renameName('a', ''));
t('deleteName empty', fn() => $z->deleteName(''));
t('deleteName missing', fn() => $z->deleteName('zz'));
t('statName empty', fn() => $z->statName(''));
t('locateName empty', fn() => $z->locateName(''));
t('getFromName empty', fn() => $z->getFromName(''));
t('getFromName negative len', fn() => $z->getFromName('a', -1));
t('getFromIndex negative', fn() => $z->getFromIndex(-1));
t('getFromIndex pending', fn() => $z->getFromIndex(0));
t('getStream empty', fn() => $z->getStream(''));
t('getStreamName empty', fn() => $z->getStreamName(''));
t('getStreamIndex bad', fn() => $z->getStreamIndex(99));
t('getStream pending', fn() => is_resource($z->getStream('a')));
t('setCommentName empty', fn() => $z->setCommentName('', 'c'));
t('setCommentName missing', fn() => $z->setCommentName('zz', 'c'));
t('setCommentIndex long', fn() => $z->setCommentIndex(0, str_repeat('x', 70000)));
t('setArchiveComment long', fn() => $z->setArchiveComment(str_repeat('x', 70000)));
t('getCommentName empty', fn() => $z->getCommentName(''));
t('getCommentIndex bad', fn() => $z->getCommentIndex(99));
t('setExternalAttributesName empty', fn() => $z->setExternalAttributesName('', 0, 0));
t('getExternalAttributesName empty', fn() => $z->getExternalAttributesName('', $a, $b));
t('getExternalAttributesIndex bad', fn() => $z->getExternalAttributesIndex(99, $a, $b));
t('setCompressionName empty', fn() => $z->setCompressionName('', 0));
t('setCompressionName unknown', fn() => $z->setCompressionName('a', 99));
t('pending stat after setCompression', fn() => [$z->setCompressionIndex(0, ZipArchive::CM_STORE), array_diff_key($z->statIndex(0), ['mtime' => 1])]);
t('setMtimeName empty', fn() => $z->setMtimeName('', 1));
t('unchangeName empty', fn() => $z->unchangeName(''));
t('unchangeName missing', fn() => $z->unchangeName('zz'));
t('unchangeIndex bad', fn() => $z->unchangeIndex(99));
t('extractTo empty', fn() => $z->extractTo(''));
t('addGlob empty', fn() => $z->addGlob(''));
t('addPattern empty', fn() => $z->addPattern(''));
t('addGlob bad flags', fn() => $z->addGlob('*', 1 << 29));
t('addGlob bad option type', fn() => $z->addGlob('*.nomatch', 0, ['add_path' => 5]));
t('addGlob empty option', fn() => $z->addGlob('*.nomatch', 0, ['remove_path' => '']));
t('addGlob nomatch', fn() => $z->addGlob('*.nomatch'));
t('addPattern bad regex', fn() => $z->addPattern('/unterminated'));
t('addPattern nomatch', fn() => $z->addPattern('/\.nomatch$/'));
t('setPassword empty', fn() => $z->setPassword(''));
t('getArchiveFlag unknown', fn() => $z->getArchiveFlag(99));
t('setArchiveFlag unknown', fn() => $z->setArchiveFlag(99, 1));
t('clearError', function () use ($z) { $z->statIndex(99); $a = $z->status; $z->clearError(); return [$a, $z->status, $z->getStatusString()]; });
t('Countable', fn() => count($z));
t('lastId after failure', function () use ($z) { $z->addFromString('a', 'y', 0); return $z->lastId; });
t('statIndex unchanged on new entry', fn() => $z->statIndex(0, ZipArchive::FL_UNCHANGED));
t('close', fn() => $z->close());
t('close again', fn() => $z->close());
t('closed status', fn() => [$z->status, $z->numFiles, $z->filename]);
t('wrong type', fn() => $z->addFromString([], 'x'));

unlink('data.txt');
unlink('e.zip');
chdir('/');
rmdir($base);
