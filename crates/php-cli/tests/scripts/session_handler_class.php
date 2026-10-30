<?php
// SessionHandler wraps the save handler that was in effect (files): a
// subclass changes what is written, and parent::* needs an open session.
ob_start();
ini_set('session.gc_probability', '0');
$dir = sys_get_temp_dir() . '/p8_session_class_' . getmypid();
mkdir($dir);
session_save_path($dir);

$h = new SessionHandler();
var_dump($h instanceof SessionHandlerInterface, $h instanceof SessionIdInterface, $h instanceof SessionUpdateTimestampHandlerInterface);
foreach (['open' => ['', ''], 'close' => [], 'read' => ['x'], 'write' => ['x', 'y'], 'destroy' => ['x'], 'gc' => [1], 'create_sid' => []] as $method => $args) {
    try {
        var_dump($h->$method(...$args));
    } catch (Error $e) {
        echo $method, ': ', get_class($e), ': ', $e->getMessage(), "\n";
    }
}

class Logged extends SessionHandler
{
    public function open(string $path, string $name): bool
    {
        echo "open\n";
        return parent::open($path, $name);
    }
    public function read(string $id): string|false
    {
        $data = parent::read($id);
        echo "read: ", var_export($data, true), "\n";
        return $data;
    }
    public function write(string $id, string $data): bool
    {
        echo "write: $data\n";
        return parent::write($id, $data);
    }
    public function close(): bool
    {
        echo "close\n";
        return parent::close();
    }
    public function create_sid(): string
    {
        return 'fixed-' . parent::create_sid();
    }
}

var_dump(session_set_save_handler(new Logged(), true));
var_dump(session_module_name());
session_id('p8class1');
var_dump(session_start());
var_dump(session_create_id() !== '');
$_SESSION['k'] = 'v';
session_write_close();
var_dump(file_get_contents("$dir/sess_p8class1"));
session_start();
var_dump(session_encode());
session_abort();

echo "-- parent calls before the session is open\n";
try {
    var_dump((new SessionHandler())->read('x'));
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
echo "-- the base class used as a handler\n";
var_dump(session_set_save_handler(new SessionHandler()));
session_id('p8class2');
var_dump(session_start());
$_SESSION['n'] = 1;
var_dump(session_write_close());
var_dump(file_get_contents("$dir/sess_p8class2"));

foreach (glob("$dir/sess_*") as $f) {
    unlink($f);
}
rmdir($dir);
