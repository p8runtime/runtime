<?php
// The finfo class and functions: flags, errors, files, directories, streams.
set_error_handler(function ($n, $s) {
    echo "[W: ", preg_replace('~/[^ )]*p8fi_\d+~', 'DIR', $s), "] ";
    return true;
});
function show($label, $v) {
    echo "== $label: ", var_export($v, true), "\n";
}
function attempt($label, callable $f) {
    try {
        show($label, $f());
    } catch (Throwable $e) {
        echo "== $label: ", get_class($e), ': ', preg_replace('~/[^ )]*p8fi_\d+~', 'DIR', $e->getMessage()), "\n";
    }
}

show('loaded', extension_loaded('fileinfo'));
show('constants', [FILEINFO_NONE, FILEINFO_SYMLINK, FILEINFO_MIME, FILEINFO_MIME_TYPE, FILEINFO_MIME_ENCODING, FILEINFO_DEVICES, FILEINFO_CONTINUE, FILEINFO_PRESERVE_ATIME, FILEINFO_RAW, FILEINFO_APPLE, FILEINFO_EXTENSION]);
show('methods', get_class_methods('finfo'));
show('functions', array_map('function_exists', ['finfo_open', 'finfo_close', 'finfo_set_flags', 'finfo_file', 'finfo_buffer', 'mime_content_type']));

$dir = sys_get_temp_dir() . '/p8fi_' . getmypid();
mkdir($dir);
$png = "\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x10\x00\x00\x00\x10\x08\x06\x00\x00\x00";
file_put_contents("$dir/a.png", $png);
file_put_contents("$dir/b.txt", "plain text\n");
file_put_contents("$dir/c.php", "<?php echo 1;\n");
file_put_contents("$dir/empty", '');
file_put_contents("$dir/one", 'x');
file_put_contents("$dir/big.bin", str_repeat("\x00\x01\x02", 1000));

// the object API
$f = new finfo(FILEINFO_MIME_TYPE);
show('file png', $f->file("$dir/a.png"));
show('file txt', $f->file("$dir/b.txt"));
show('file php', $f->file("$dir/c.php"));
show('file empty', $f->file("$dir/empty"));
show('file one', $f->file("$dir/one"));
show('file big', $f->file("$dir/big.bin"));
show('file dir', $f->file($dir));
show('file flags', $f->file("$dir/a.png", FILEINFO_NONE));
show('file mime', $f->file("$dir/a.png", FILEINFO_MIME));
show('file again', $f->file("$dir/a.png"));
attempt('file missing', fn() => $f->file("$dir/missing"));
attempt('file empty name', fn() => $f->file(''));
attempt('file nul', fn() => $f->file("a\0b"));
attempt('file type', fn() => $f->file([]));
show('buffer', $f->buffer($png));
show('buffer flags', $f->buffer($png, FILEINFO_MIME_ENCODING));
show('set_flags', $f->set_flags(FILEINFO_NONE));
show('after set_flags', $f->buffer($png));
show('set_flags enc', $f->set_flags(FILEINFO_MIME_ENCODING));
show('after enc', $f->buffer("<?php echo 1;"));
attempt('set_flags type', fn() => $f->set_flags('x'));
attempt('construct type', fn() => new finfo('x'));
attempt('construct many', fn() => new finfo(0, null, 3));
attempt('buffer none', fn() => $f->buffer());

// the procedural API
$h = finfo_open(FILEINFO_MIME);
show('is object', $h instanceof finfo);
show('finfo_file', finfo_file($h, "$dir/c.php"));
show('finfo_file flags', finfo_file($h, "$dir/c.php", FILEINFO_MIME_TYPE));
show('finfo_buffer', finfo_buffer($h, "GIF89a\x10\x00\x10\x00"));
show('finfo_buffer flags', finfo_buffer($h, "GIF89a\x10\x00\x10\x00", FILEINFO_NONE));
show('finfo_set_flags', finfo_set_flags($h, FILEINFO_MIME_TYPE));
show('finfo_buffer after', finfo_buffer($h, "GIF89a\x10\x00\x10\x00"));
attempt('finfo_file missing', fn() => finfo_file($h, "$dir/missing"));
attempt('finfo_file empty', fn() => finfo_file($h, ''));
attempt('finfo_file nul', fn() => finfo_file($h, "a\0b"));
attempt('finfo_file notobj', fn() => finfo_file('x', 'y'));
show('finfo_close', finfo_close($h));
show('default open', get_class(finfo_open()));
show('default flags', finfo_buffer(finfo_open(), 'text'));
show('open any flags', get_class(finfo_open(-1)));

// mime_content_type
show('mct png', mime_content_type("$dir/a.png"));
show('mct txt', mime_content_type("$dir/b.txt"));
show('mct empty', mime_content_type("$dir/empty"));
show('mct dir', mime_content_type($dir));
attempt('mct missing', fn() => mime_content_type("$dir/missing"));
attempt('mct empty name', fn() => mime_content_type(''));
attempt('mct nul', fn() => mime_content_type("a\0b"));
attempt('mct int', fn() => mime_content_type(1));
attempt('mct null', fn() => mime_content_type(null));
attempt('mct array', fn() => mime_content_type([]));
$fp = fopen("$dir/a.png", 'rb');
fseek($fp, 5);
show('mct stream', mime_content_type($fp));
show('stream pos kept', ftell($fp));
fclose($fp);

// a custom magic file
$magic = "$dir/magic";
file_put_contents($magic, "0\tstring\tFOO\tthe foo format\n!:mime\tapplication/x-foo\n>3\tbyte\tx\t\\b, byte %d\n");
$m = new finfo(FILEINFO_NONE, $magic);
show('custom', $m->buffer("FOO\x07"));
$m = new finfo(FILEINFO_MIME, $magic);
show('custom mime', $m->buffer("FOO\x07"));
show('custom other', $m->buffer("BAR"));
attempt('magic missing', fn() => new finfo(FILEINFO_NONE, "$dir/nomagic"));
show('open magic missing', finfo_open(FILEINFO_NONE, "$dir/nomagic"));

// FILEINFO_EXTENSION and FILEINFO_APPLE
$e = new finfo(FILEINFO_EXTENSION);
show('ext png', $e->buffer($png));
show('ext text', $e->buffer("plain text\n"));
show('ext unknown', $e->buffer("\x01\x02\x03\x04"));
$a = new finfo(FILEINFO_APPLE);
show('apple unknown', $a->buffer("\x01\x02\x03\x04"));

foreach (glob("$dir/*") as $x) {
    unlink($x);
}
rmdir($dir);
