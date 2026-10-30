<?php
// A session written when the request ends reports what goes wrong there
// without a function or a file to blame.
ini_set('session.gc_probability', '0');
ini_set('session.cache_limiter', '');

class Failing implements SessionHandlerInterface
{
    public function open(string $path, string $name): bool { return true; }
    public function close(): bool { return true; }
    public function read(string $id): string|false { return ''; }
    public function write(string $id, string $data): bool { return false; }
    public function destroy(string $id): bool { return true; }
    public function gc(int $max_lifetime): int|false { return 0; }
}

session_set_save_handler(new Failing(), false);
session_id('p8enderr');
session_start();
$_SESSION['x'] = 1;
echo "end of script\n";
