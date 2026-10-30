<?php
// The session is written when the request ends: after the shutdown
// functions and the destructors, unless session_set_save_handler() made
// session_write_close() a shutdown function of its own, which then runs in
// its place among them.
ini_set('session.gc_probability', '0');
ini_set('session.cache_limiter', '');

class Handler implements SessionHandlerInterface
{
    public function open(string $path, string $name): bool { echo "open\n"; return true; }
    public function close(): bool { echo "close\n"; return true; }
    public function read(string $id): string|false { echo "read\n"; return ''; }
    public function write(string $id, string $data): bool { echo "write($data)\n"; return true; }
    public function destroy(string $id): bool { return true; }
    public function gc(int $max_lifetime): int|false { return 0; }
}

class Dtor
{
    public function __destruct() { echo "destructor\n"; }
}

$dtor = new Dtor();
ob_start(function ($buffer) {
    return "[buffer: " . strlen($buffer) . " bytes]\n" . $buffer;
});

register_shutdown_function(function () {
    echo "shutdown 1: session ", session_status() === PHP_SESSION_ACTIVE ? "active" : "closed", "\n";
});
session_set_save_handler(new Handler());
register_shutdown_function(function () {
    echo "shutdown 2: session ", session_status() === PHP_SESSION_ACTIVE ? "active" : "closed", "\n";
});
session_id('p8end');
session_start();
$_SESSION['x'] = 1;
echo "end of script\n";
