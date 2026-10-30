<?php
// A wrapper whose methods all go through __call (as Laravel's tests fake a
// socket with one), a by-reference opened path, the context property, and
// stream_select on user-space streams.
class FakeStream {
    public static array $instances = [];
    public $context;
    public string $stream = '';

    public function __call(string $name, array $arguments): mixed {
        $handler = match ($name) {
            'stream_open' => function (string $path, string $mode, int $options, ?string &$openedPath): bool {
                self::$instances[] = $this;
                $openedPath = $path;
                return true;
            },
            'stream_set_option' => fn (int $option, int $arg1, ?int $arg2): bool => true,
            'stream_write' => function (string $chunk): int {
                $this->stream .= $chunk;
                return strlen($chunk);
            },
            'stream_read' => fn (int $length): string|false => '2:OK',
            'stream_eof' => fn (): bool => false,
            'stream_flush' => fn (): bool => true,
            'stream_close' => function (): void {
                echo "closed\n";
            },
            'stream_seek' => fn () => 0,
            'url_stat' => fn (string $path, int $flags): array|false => false,
            default => throw new RuntimeException("FakeStream method not implemented [{$name}]"),
        };
        return call_user_func_array($handler, $arguments);
    }
}

try {
    var_dump(stream_wrapper_register('unix', FakeStream::class));
} catch (Throwable $e) {
    echo get_class($e), "\n";
}
var_dump(in_array('unix', stream_get_wrappers()));
$ctx = stream_context_create(['unix' => ['x' => 1]]);
$socket = fopen('unix:///tmp/fake.sock', 'r+', false, $ctx);
var_dump(is_resource($socket));
var_dump(count(FakeStream::$instances));
var_dump(stream_context_get_options(FakeStream::$instances[0]->context));
var_dump(stream_set_timeout($socket, 1));
var_dump(fwrite($socket, '{"a":1}' . "\n"));
var_dump(fwrite($socket, '{"b":2}' . "\n"));
var_dump(fread($socket, 4));
var_dump(fflush($socket));
var_dump(FakeStream::$instances[0]->stream);
var_dump(file_exists('unix:///tmp/fake.sock'));
try {
    fstat($socket);
} catch (RuntimeException $e) {
    echo $e->getMessage(), "\n";
}
var_dump(fclose($socket));

class Pipe {
    public $context;
    public static $inner;
    function stream_open($path, $mode, $options, &$opened) { return true; }
    function stream_cast($as) { echo "cast $as\n"; return self::$inner ?? false; }
    function stream_read($n) { return ''; }
    function stream_eof() { return true; }
}
stream_wrapper_register('pipe', 'Pipe');
function select_on($p) {
    $r = [$p];
    $w = $x = null;
    try {
        var_dump(stream_select($r, $w, $x, 0));
        var_dump($r === [$p]);
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
}
Pipe::$inner = fopen('php://stdin', 'r');
$p = fopen('pipe://a', 'r');
select_on($p);
Pipe::$inner = null;
select_on($p);
Pipe::$inner = $p;
select_on($p);
Pipe::$inner = 'nope';
select_on($p);
