<?php
// A user save handler (SessionHandlerInterface): the order of its calls,
// lazy writes, validation, the return values it is held to, and the
// SessionHandler it can wrap.
ob_start();
// (no collection by chance)
ini_set('session.gc_probability', '0');

class Logging implements SessionHandlerInterface, SessionIdInterface, SessionUpdateTimestampHandlerInterface
{
    public array $store = [];
    public function open(string $path, string $name): bool { echo "open($name)\n"; return true; }
    public function close(): bool { echo "close\n"; return true; }
    public function read(string $id): string|false { echo "read($id)\n"; return $this->store[$id] ?? ''; }
    public function write(string $id, string $data): bool { echo "write($id, $data)\n"; $this->store[$id] = $data; return true; }
    public function destroy(string $id): bool { echo "destroy($id)\n"; unset($this->store[$id]); return true; }
    public function gc(int $max_lifetime): int|false { echo "gc\n"; return 3; }
    public function create_sid(): string { echo "create_sid\n"; return 'created-id'; }
    public function validateId(string $id): bool { echo "validateId($id)\n"; return isset($this->store[$id]); }
    public function updateTimestamp(string $id, string $data): bool { echo "updateTimestamp($id, $data)\n"; return true; }
}

$h = new Logging();
var_dump(session_set_save_handler($h, false));
var_dump(session_module_name(), ini_get('session.save_handler'));
var_dump(session_start());
var_dump(session_id());
$_SESSION['a'] = 1;
var_dump(session_gc());
var_dump(session_write_close());

echo "-- the same data: only the timestamp\n";
session_id('created-id');
session_start();
session_write_close();

echo "-- changed data: written\n";
session_start();
$_SESSION['a'] = 2;
session_write_close();

echo "-- strict mode: an id that does not exist is replaced\n";
ini_set('session.use_strict_mode', '1');
session_id('nothere');
session_start();
var_dump(session_id());
session_write_close();

echo "-- regenerate\n";
session_id('created-id');
session_start();
var_dump(session_regenerate_id(true));
var_dump(session_id());
session_destroy();

echo "-- create_id inside an active session asks the handler\n";
session_start();
var_dump(session_create_id('x-'));
session_abort();

echo "-- collection by chance runs after the read\n";
ini_set('session.gc_probability', '1');
ini_set('session.gc_divisor', '1');
session_id('created-id');
session_start();
session_abort();
ini_set('session.gc_probability', '0');

echo "-- recursive calls are refused\n";
class Recursive extends Logging
{
    public function read(string $id): string|false
    {
        var_dump(session_start());
        return '';
    }
}
session_set_save_handler(new Recursive(), false);
session_id('abc');
try {
    session_start();
} catch (Throwable $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
session_abort();

echo "-- what a handler returns\n";
class Returns extends Logging
{
    public $ret;
    public function open(string $path, string $name): bool { return $this->ret; }
}
$r = new Returns();
session_set_save_handler($r, false);
foreach ([false, true] as $ret) {
    $r->ret = $ret;
    session_id('r1');
    var_dump(@session_start());
    session_abort();
}
