<?php
// WRITEFUNCTION, HEADERFUNCTION, aborting, exceptions, callbacks and the handle.

$base = getenv("CURL_BASE");

echo "--- write and header functions\n";
$ch = curl_init("$base/headers");
$lines = [];
curl_setopt($ch, CURLOPT_HEADERFUNCTION, function ($c, $h) use (&$lines, $ch) {
    var_dump($c === $ch);
    $lines[] = $h;
    return strlen($h);
});
curl_setopt($ch, CURLOPT_WRITEFUNCTION, function ($c, $d) {
    echo "body: ", json_encode($d), "\n";
    return strlen($d);
});
var_dump(curl_exec($ch));
echo json_encode($lines), "\n";

echo "--- write function aborts\n";
$ch = curl_init("$base/headers");
curl_setopt($ch, CURLOPT_WRITEFUNCTION, fn($c, $d) => 0);
var_dump(curl_exec($ch), curl_errno($ch));
echo preg_replace('/\d+/', 'N', curl_error($ch)), "\n";

echo "--- header function aborts\n";
$ch = curl_init("$base/headers");
curl_setopt($ch, CURLOPT_HEADERFUNCTION, fn($c, $h) => 1);
var_dump(curl_exec($ch), curl_errno($ch));

echo "--- exception in a callback\n";
$ch = curl_init("$base/headers");
curl_setopt($ch, CURLOPT_WRITEFUNCTION, function ($c, $d) {
    throw new RuntimeException("from callback");
});
try {
    curl_exec($ch);
} catch (RuntimeException $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

echo "--- close and reset in a callback\n";
$ch = curl_init("$base/headers");
curl_setopt($ch, CURLOPT_WRITEFUNCTION, function ($c, $d) {
    foreach (["curl_close", "curl_reset"] as $f) {
        try {
            $f($c);
        } catch (Error $e) {
            echo $e->getMessage(), "\n";
        }
    }
    return strlen($d);
});
var_dump(curl_exec($ch));

echo "--- setopt in a callback\n";
$ch = curl_init("$base/headers");
curl_setopt($ch, CURLOPT_WRITEFUNCTION, function ($c, $d) {
    curl_setopt($c, CURLOPT_WRITEFUNCTION, null);
    echo "first\n";
    return strlen($d);
});
var_dump(curl_exec($ch));

echo "--- big body in pieces\n";
$ch = curl_init("$base/big");
$total = 0;
$calls = 0;
curl_setopt($ch, CURLOPT_WRITEFUNCTION, function ($c, $d) use (&$total, &$calls) {
    $total += strlen($d);
    $calls++;
    return strlen($d);
});
var_dump(curl_exec($ch), $total, $calls > 1);

echo "--- callbacks with a method and a static\n";
class Sink {
    public string $data = "";
    public function write($ch, $d) { $this->data .= $d; return strlen($d); }
    public static function head($ch, $h) { return strlen($h); }
}
$sink = new Sink;
$ch = curl_init("$base/chunked");
curl_setopt($ch, CURLOPT_WRITEFUNCTION, [$sink, "write"]);
curl_setopt($ch, CURLOPT_HEADERFUNCTION, "Sink::head");
var_dump(curl_exec($ch), $sink->data);

echo "--- private data and info\n";
curl_setopt($ch, CURLOPT_PRIVATE, "tag");
var_dump(curl_getinfo($ch, CURLINFO_PRIVATE), curl_getinfo($ch, CURLINFO_HTTP_CODE));

echo "--- copy keeps options\n";
$ch = curl_init("$base/echo");
curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_HTTPHEADER => ["X-Copy: yes"]]);
$copy = curl_copy_handle($ch);
$r = curl_exec($copy);
echo strtok($r, "\n"), "\n", preg_match('/x-copy: yes/', $r), "\n";
$clone = clone $ch;
echo preg_match('/x-copy: yes/', curl_exec($clone)), "\n";
