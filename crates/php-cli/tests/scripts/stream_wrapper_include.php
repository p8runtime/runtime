<?php
// include and require through a user-space wrapper: what it is asked, the
// code it serves running, the _once forms, and failures.
class Code {
    public static array $files = [];
    public $context;
    private string $data = '';
    private int $pos = 0;
    function stream_open($path, $mode, $options, &$opened) {
        echo "open $path $mode $options ", var_export($this->context, true), "\n";
        if (!isset(self::$files[$path])) return false;
        $this->data = self::$files[$path];
        if (str_contains($path, 'named')) $opened = 'code://the-name.php';
        return true;
    }
    function stream_read($n) { echo "read $n\n"; $r = substr($this->data, $this->pos, $n); $this->pos += strlen($r); return $r; }
    function stream_eof() { return $this->pos >= strlen($this->data); }
    function stream_stat() { echo "stat\n"; return ['size' => strlen($this->data)]; }
    function stream_set_option($option, $a, $b) { echo "set_option $option ", var_export($a, true), " ", var_export($b, true), "\n"; return false; }
    function stream_close() { echo "close\n"; }
}
stream_wrapper_register('code', 'Code');
Code::$files['code://a.php'] = '<?php echo "a runs\n"; function from_a() { return "from a"; } return 42;';
Code::$files['code://b.php'] = "<?php\nclass FromB { const X = 'b'; }\nreturn [1, 2];";
Code::$files['code://named.php'] = '<?php echo basename(__FILE__), "\n";';
Code::$files['code://parse.php'] = '<?php echo "oops" +;';
Code::$files['code://big.php'] = '<?php echo strlen("' . str_repeat('y', 20000) . '"), "\n";';

var_dump(include 'code://a.php');
var_dump(from_a());
var_dump(require 'code://b.php');
var_dump(FromB::X);
var_dump(include_once 'code://a.php');
var_dump(require_once 'code://b.php');
var_dump(require_once 'code://b.php');
include 'code://named.php';
include 'code://big.php';
var_dump(include 'code://missing.php');
try {
    require 'code://missing.php';
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
try {
    include 'code://parse.php';
} catch (ParseError $e) {
    echo get_class($e), "\n";
}

// A wrapper whose methods all go through __call, throwing as it reads.
class Thrower {
    public $context;
    function stream_open() { return true; }
    function stream_read() { echo "stream_read\n"; throw new Exception("from stream_read"); }
    function __call($m, $a) { echo "$m\n"; }
}
stream_wrapper_register('throw', 'Thrower');
try {
    require 'throw://x';
} catch (Exception $e) {
    echo $e->getMessage(), "\n";
}

// A minimal wrapper: PHP warns of what is missing.
class Minimal {
    public $context;
    private $done = false;
    function stream_open($path, $mode, $options, &$opened) { return true; }
    function stream_read($n) { if ($this->done) return ''; $this->done = true; return '<?php echo "minimal\n";'; }
    function stream_eof() { return $this->done; }
}
stream_wrapper_register('min', 'Minimal');
include 'min://x';
var_dump(in_array('code://a.php', get_included_files()));
