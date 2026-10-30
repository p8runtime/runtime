<?php
// When a user-space stream's methods run: writes in chunk_size pieces,
// reads through PHP's buffer, the close an unset or the request's end
// brings, options a script sets.
class Log {
    public static array $data = [];
    public $context;
    private string $path = '';
    private int $pos = 0;
    function stream_open($path, $mode, $options, &$opened) {
        echo "open $path $mode\n";
        $this->path = $path;
        self::$data[$path] ??= '';
        return true;
    }
    function stream_write($d) { echo "write ", strlen($d), "\n"; self::$data[$this->path] .= $d; return strlen($d); }
    function stream_read($n) {
        $r = substr(self::$data[$this->path], $this->pos, min($n, 5000));
        echo "read $n -> ", strlen($r), "\n";
        $this->pos += strlen($r);
        return $r;
    }
    function stream_eof() { return $this->pos >= strlen(self::$data[$this->path]); }
    function stream_tell() { echo "tell\n"; return $this->pos; }
    function stream_seek($offset, $whence) { echo "seek $offset $whence\n"; $this->pos = $offset; return true; }
    function stream_flush() { echo "flush\n"; return true; }
    function stream_close() { echo "close {$this->path}\n"; }
    function stream_set_option($option, $arg1, $arg2) { echo "set_option $option ", var_export($arg1, true), " ", var_export($arg2, true), "\n"; return $option != STREAM_OPTION_READ_TIMEOUT; }
    function __destruct() { echo "destruct {$this->path}\n"; }
}
stream_wrapper_register('log', 'Log');

echo "-- writes\n";
$f = fopen('log://one', 'w');
var_dump(fwrite($f, str_repeat('x', 20000)));
var_dump(fwrite($f, ''));
var_dump(fwrite($f, 'abc'));
var_dump(ftell($f));
unset($f);
echo "after unset\n";

echo "-- reads\n";
$f = fopen('log://one', 'r');
var_dump(strlen(fread($f, 10)));
var_dump(strlen(fread($f, 9000)));
var_dump(ftell($f));
var_dump(strlen(fgets($f)));
var_dump(fseek($f, 100, SEEK_CUR));
var_dump(ftell($f));
var_dump(fseek($f, 15000));
var_dump(strlen(stream_get_contents($f)));
var_dump(feof($f));
var_dump(stream_set_chunk_size($f, 100));
var_dump(rewind($f));
var_dump(strlen(fread($f, 1000)));
var_dump(fclose($f));

echo "-- options\n";
$f = fopen('log://two', 'r+');
var_dump(stream_set_blocking($f, false));
var_dump(stream_set_timeout($f, 5, 10));
var_dump(stream_set_write_buffer($f, 0));
var_dump(stream_set_write_buffer($f, 4096));
var_dump(stream_set_read_buffer($f, 0));
var_dump(fwrite($f, 'hello'));
var_dump(fflush($f));
var_dump(fwrite($f, 'world'));
fclose($f);

echo "-- copy between wrappers\n";
$a = fopen('log://three', 'w+');
fwrite($a, "line1\nline2\n");
rewind($a);
$b = fopen('php://memory', 'w+');
var_dump(stream_copy_to_stream($a, $b));
rewind($b);
var_dump(stream_get_contents($b));
fclose($a);

echo "-- left open\n";
$g = fopen('log://four', 'w');
fwrite($g, 'abc');
$d = new class { function __destruct() { echo "object destruct\n"; } };
ob_start(fn ($s) => strtoupper($s));
echo "end\n";
