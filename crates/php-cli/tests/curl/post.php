<?php
// Request bodies: POSTFIELDS as string and array, multipart with files, PUT uploads.

$base = getenv("CURL_BASE");
function post($opts, $path = "/echo") {
    global $base;
    $ch = curl_init("$base$path");
    curl_setopt_array($ch, $opts + [CURLOPT_RETURNTRANSFER => true]);
    $r = curl_exec($ch);
    if ($r === false) {
        echo "error ", curl_errno($ch), ": ", curl_error($ch), "\n";
    }
    return $r;
}

echo "--- string\n";
echo post([CURLOPT_POST => true, CURLOPT_POSTFIELDS => "a=b&c=d"]);
echo "--- postfields alone\n";
echo post([CURLOPT_POSTFIELDS => "x=1"]);
echo "--- empty\n";
echo post([CURLOPT_POST => true, CURLOPT_POSTFIELDS => ""]);
echo "--- custom content type\n";
echo post([CURLOPT_POSTFIELDS => '{"a":1}', CURLOPT_HTTPHEADER => ["Content-Type: application/json"]]);
echo "--- array\n";
echo post([CURLOPT_POSTFIELDS => ["name" => "v1", "list" => ["a", "b"], 5 => "five", "n" => 42]]);
echo "--- files\n";
$tmp = tempnam(sys_get_temp_dir(), "cf");
file_put_contents($tmp, "line1\nline2\n");
echo post([CURLOPT_POSTFIELDS => [
    "plain" => "text",
    "file" => new CURLFile($tmp, "text/plain", "up.txt"),
    "default_mime" => new CURLFile($tmp, null, "d.txt"),
    "str" => new CURLStringFile("in memory", "mem.bin", "application/x-test"),
]]);
unlink($tmp);
echo "--- post to get\n";
echo post([CURLOPT_POST => true, CURLOPT_POSTFIELDS => "gone"] + [CURLOPT_HTTPGET => true]);
echo "--- put\n";
$fp = fopen("php://memory", "w+");
fwrite($fp, "uploaded body");
rewind($fp);
echo post([CURLOPT_UPLOAD => true, CURLOPT_INFILE => $fp, CURLOPT_INFILESIZE => 13]);
echo "\n--- put with read function\n";
$parts = ["first ", "second ", "third"];
echo post([CURLOPT_UPLOAD => true, CURLOPT_INFILESIZE => 18, CURLOPT_READFUNCTION => function ($ch, $stream, $len) use (&$parts) {
    return array_shift($parts) ?? "";
}]);
echo "\n--- custom method with body\n";
echo post([CURLOPT_CUSTOMREQUEST => "PATCH", CURLOPT_POSTFIELDS => "patched"]);
