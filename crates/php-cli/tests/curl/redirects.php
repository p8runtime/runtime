<?php
// Redirects, errors and timeouts.

$base = getenv("CURL_BASE");
function clean($s) { global $base; $h = parse_url($base, PHP_URL_HOST) . ":" . parse_url($base, PHP_URL_PORT); return str_replace([$base, $h], ["BASE", "HOST"], $s); }

echo "--- not followed\n";
$ch = curl_init("$base/r/302/echo");
curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true]);
var_dump(curl_exec($ch), curl_getinfo($ch, CURLINFO_HTTP_CODE), clean(curl_getinfo($ch, CURLINFO_REDIRECT_URL)), curl_getinfo($ch, CURLINFO_REDIRECT_COUNT));

echo "--- followed\n";
curl_setopt($ch, CURLOPT_FOLLOWLOCATION, true);
$r = curl_exec($ch);
echo strtok($r, "\n"), "\n";
var_dump(curl_getinfo($ch, CURLINFO_HTTP_CODE), clean(curl_getinfo($ch, CURLINFO_EFFECTIVE_URL)), curl_getinfo($ch, CURLINFO_REDIRECT_COUNT), curl_getinfo($ch, CURLINFO_REDIRECT_URL));

echo "--- method after redirect\n";
foreach ([301, 302, 303, 307, 308] as $code) {
    $ch = curl_init("$base/r/$code/echo");
    curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_FOLLOWLOCATION => true, CURLOPT_POSTFIELDS => "payload"]);
    $r = curl_exec($ch);
    echo $code, " ", strtok($r, "\n"), " ", str_contains($r, "payload") ? "body kept" : "no body", "\n";
}
$ch = curl_init("$base/r/302/echo");
curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_FOLLOWLOCATION => true, CURLOPT_POSTFIELDS => "payload", CURLOPT_POSTREDIR => CURL_REDIR_POST_302]);
echo strtok(curl_exec($ch), "\n"), "\n";

echo "--- headers of every hop\n";
$ch = curl_init("$base/r/301/headers");
curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_FOLLOWLOCATION => true, CURLOPT_HEADER => true]);
$r = curl_exec($ch);
echo substr_count($r, "HTTP/1.1"), " status lines, header size ", curl_getinfo($ch, CURLINFO_HEADER_SIZE), "\n";

echo "--- too many redirects\n";
$ch = curl_init("$base/loop");
curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_FOLLOWLOCATION => true, CURLOPT_MAXREDIRS => 3]);
var_dump(curl_exec($ch), curl_errno($ch), curl_error($ch), curl_getinfo($ch, CURLINFO_REDIRECT_COUNT));
curl_setopt($ch, CURLOPT_MAXREDIRS, 0);
var_dump(curl_exec($ch), curl_errno($ch), curl_error($ch));

echo "--- autoreferer\n";
$ch = curl_init("$base/r/302/echo");
curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_FOLLOWLOCATION => true, CURLOPT_AUTOREFERER => true]);
echo clean(curl_exec($ch));

echo "--- connection refused\n";
$ch = curl_init("http://127.0.0.1:1/");
var_dump(curl_exec($ch), curl_errno($ch));
echo preg_replace('/after \d+ ms/', 'after N ms', curl_error($ch)), "\n";
var_dump(curl_strerror(curl_errno($ch)));

echo "--- unresolvable host\n";
$ch = curl_init("http://no-such-host.invalid/");
var_dump(curl_exec($ch), curl_errno($ch), curl_error($ch));

echo "--- timeout\n";
$ch = curl_init("$base/slow");
curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_TIMEOUT_MS => 300]);
$t = microtime(true);
var_dump(curl_exec($ch), curl_errno($ch));
echo preg_replace('/\d+ milliseconds/', 'N milliseconds', curl_error($ch)), "\n";
var_dump(microtime(true) - $t < 1.2);
echo "--- not a timeout\n";
curl_setopt($ch, CURLOPT_TIMEOUT, 10);
curl_setopt($ch, CURLOPT_TIMEOUT_MS, 0);
var_dump(curl_exec($ch));

echo "--- bad urls\n";
foreach (["nosuch://x/", "://nothing", "http://"] as $u) {
    $ch = curl_init($u);
    var_dump(curl_exec($ch), curl_errno($ch));
}

echo "--- error state resets\n";
$ch = curl_init("http://127.0.0.1:1/");
curl_exec($ch);
var_dump(curl_errno($ch) > 0);
curl_setopt($ch, CURLOPT_URL, "$base/echo");
curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
var_dump(is_string(curl_exec($ch)), curl_errno($ch), curl_error($ch));

echo "--- multi\n";
$mh = curl_multi_init();
$handles = [];
foreach (["echo?n=1", "headers", "status/404"] as $i => $path) {
    $handles[$i] = curl_init("$base/$path");
    curl_setopt($handles[$i], CURLOPT_RETURNTRANSFER, true);
    curl_multi_add_handle($mh, $handles[$i]);
}
$bad = curl_init("http://127.0.0.1:1/");
curl_multi_add_handle($mh, $bad);
do {
    $status = curl_multi_exec($mh, $running);
    if ($running) {
        curl_multi_select($mh);
    }
} while ($running > 0 && $status === CURLM_OK);
var_dump($status);
$results = [];
while ($info = curl_multi_info_read($mh)) {
    $results[] = [array_search($info["handle"], [...$handles, $bad], true), $info["msg"], $info["result"]];
}
sort($results);
echo json_encode($results), "\n";
foreach ($handles as $h) {
    echo strtok(curl_multi_getcontent($h), "\n"), "|", curl_getinfo($h, CURLINFO_HTTP_CODE), "\n";
    curl_multi_remove_handle($mh, $h);
}
var_dump(curl_error($bad) !== "", curl_multi_info_read($mh));
curl_multi_close($mh);
