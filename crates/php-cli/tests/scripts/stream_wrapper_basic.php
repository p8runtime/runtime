<?php
class Mem {
    public static array $files = [];
    public static array $dirs = ['mem://d' => true];
    public $context;
    private string $path = '';
    private int $pos = 0;
    private array $list = [];
    private int $di = 0;
    function __construct() { echo "__construct\n"; }
    function stream_open($path, $mode, $options, &$opened) {
        echo "stream_open($path, $mode, $options) ctx=", get_debug_type($this->context), "\n";
        $this->path = $path;
        if ($mode[0] === 'r' && !isset(self::$files[$path])) return false;
        if ($mode[0] === 'w') self::$files[$path] = '';
        if ($mode[0] === 'a') { self::$files[$path] ??= ''; $this->pos = strlen(self::$files[$path]); }
        if ($mode[0] === 'x') { if (isset(self::$files[$path])) return false; self::$files[$path] = ''; }
        if ($mode[0] === 'c') { self::$files[$path] ??= ''; }
        return true;
    }
    function stream_read($n) { echo "stream_read($n)\n"; $r = substr(self::$files[$this->path], $this->pos, $n); $this->pos += strlen($r); return $r; }
    function stream_write($d) { echo "stream_write(", strlen($d), ")\n"; $f = &self::$files[$this->path]; $f = substr($f, 0, $this->pos) . $d . substr($f, $this->pos + strlen($d)); $this->pos += strlen($d); return strlen($d); }
    function stream_eof() { echo "stream_eof\n"; return $this->pos >= strlen(self::$files[$this->path]); }
    function stream_tell() { echo "stream_tell\n"; return $this->pos; }
    function stream_seek($o, $w) { echo "stream_seek($o, $w)\n"; $len = strlen(self::$files[$this->path]); $p = match ($w) { SEEK_SET => $o, SEEK_CUR => $this->pos + $o, SEEK_END => $len + $o }; if ($p < 0) return false; $this->pos = $p; return true; }
    function stream_close() { echo "stream_close\n"; }
    function stream_flush() { echo "stream_flush\n"; return true; }
    function stream_stat() { echo "stream_stat\n"; return ['size' => strlen(self::$files[$this->path]), 'mode' => 0100644]; }
    function stream_truncate(int $n) { echo "stream_truncate($n)\n"; $f = &self::$files[$this->path]; $f = str_pad(substr($f, 0, $n), $n, "\0"); return true; }
    function stream_lock($op) { echo "stream_lock($op)\n"; return true; }
    function stream_set_option($o, $a, $b) { echo "stream_set_option($o, ", var_export($a, true), ", ", var_export($b, true), ")\n"; return false; }
    function stream_metadata($path, $option, $value) { echo "stream_metadata($path, $option, ", json_encode($value), ")\n"; if ($option == STREAM_META_TOUCH) { self::$files[$path] ??= ''; } return true; }
    function url_stat($path, $flags) {
        echo "url_stat($path, $flags) ctx=", get_debug_type($this->context), "\n";
        if (isset(self::$dirs[$path])) return ['mode' => 040755, 'size' => 0, 'mtime' => 100];
        if (!isset(self::$files[$path])) return false;
        return ['mode' => 0100644, 'size' => strlen(self::$files[$path]), 'mtime' => 1234567890, 7 => 99];
    }
    function unlink($path) { echo "unlink($path) ctx=", get_debug_type($this->context), "\n"; if (!isset(self::$files[$path])) return false; unset(self::$files[$path]); return true; }
    function rename($a, $b) { echo "rename($a, $b)\n"; self::$files[$b] = self::$files[$a]; unset(self::$files[$a]); return true; }
    function mkdir($path, $mode, $options) { echo "mkdir($path, ", decoct($mode), ", $options)\n"; self::$dirs[$path] = true; return true; }
    function rmdir($path, $options) { echo "rmdir($path, $options)\n"; unset(self::$dirs[$path]); return true; }
    function dir_opendir($path, $options) { echo "dir_opendir($path, $options)\n"; $this->list = ['a', 'b', 'c']; return isset(self::$dirs[$path]); }
    function dir_readdir() { echo "dir_readdir\n"; return $this->list[$this->di++] ?? false; }
    function dir_rewinddir() { echo "dir_rewinddir\n"; $this->di = 0; return true; }
    function dir_closedir() { echo "dir_closedir\n"; return true; }
}
var_dump(stream_wrapper_register('mem', 'Mem'));
echo "-- put\n";
var_dump(file_put_contents('mem://a', "hello\nworld\n"));
echo "-- get\n";
var_dump(file_get_contents('mem://a'));
echo "-- file\n";
var_dump(file('mem://a', FILE_IGNORE_NEW_LINES));
echo "-- fopen r\n";
$f = fopen('mem://a', 'r');
var_dump(fgets($f));
var_dump(ftell($f));
var_dump(feof($f));
var_dump(fread($f, 3));
var_dump(fseek($f, 1));
var_dump(fread($f, 4));
var_dump(fstat($f)['size']);
var_dump(flock($f, LOCK_EX));
var_dump(fclose($f));
echo "-- fopen r+\n";
$f = fopen('mem://a', 'r+');
var_dump(fwrite($f, "HE"));
var_dump(fflush($f));
var_dump(ftruncate($f, 4));
var_dump(rewind($f));
var_dump(stream_get_contents($f));
var_dump(fclose($f));
echo "-- missing\n";
var_dump(fopen('mem://nope', 'r'));
var_dump(file_get_contents('mem://nope'));
echo "-- stat\n";
var_dump(file_exists('mem://a'), is_file('mem://a'), is_dir('mem://a'), is_dir('mem://d'), filesize('mem://a'), filemtime('mem://a'));
var_dump(file_exists('mem://nope'));
var_dump(filesize('mem://nope'));
var_dump(stat('mem://a')['size']);
var_dump(stream_get_meta_data(fopen("mem://b", "w"))["wrapper_type"]);
echo "-- ops\n";
var_dump(rename('mem://a', 'mem://b'));
var_dump(unlink('mem://b'));
var_dump(unlink('mem://b'));
var_dump(mkdir('mem://e', 0700, true));
var_dump(rmdir('mem://e'));
var_dump(touch('mem://t', 5, 6));
var_dump(touch('mem://t'));
var_dump(chmod('mem://t', 0600));
echo "-- dir\n";
$d = opendir('mem://d');
var_dump(readdir($d), readdir($d));
rewinddir($d);
var_dump(readdir($d));
closedir($d);
var_dump(scandir('mem://d'));
var_dump(opendir('mem://zz'));
echo "-- include\n";
Mem::$files['mem://code.php'] = '<?php echo "included!\n"; return 42;';
var_dump(include 'mem://code.php');
echo "-- copy\n";
var_dump(copy('mem://code.php', 'mem://copy'));
var_dump(Mem::$files['mem://copy']);
echo "-- unregister\n";
var_dump(stream_wrapper_unregister('mem'));
var_dump(in_array('mem', stream_get_wrappers()));
var_dump(@file_get_contents('mem://copy'));
var_dump(stream_wrapper_unregister('mem'));
var_dump(stream_wrapper_restore('mem'));
var_dump(stream_wrapper_unregister('file'));
var_dump(@file_get_contents(__FILE__));
var_dump(stream_wrapper_restore('file'));
var_dump(stream_wrapper_restore('file'));
var_dump(strlen(file_get_contents('php://memory')));
