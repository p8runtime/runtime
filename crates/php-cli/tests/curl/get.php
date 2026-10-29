<?php
// GET requests: headers sent, response info, HEAD, custom methods, auth, cookies.

$base = getenv("CURL_BASE");
function clean($s) { global $base; $h = parse_url($base, PHP_URL_HOST) . ":" . parse_url($base, PHP_URL_PORT); return str_replace([$base, $h], ["BASE", "HOST"], $s); }

$ch = curl_init("$base/echo?a=1&b=two");
curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
echo curl_exec($ch);
$i = curl_getinfo($ch);
echo clean($i["url"]), " ", $i["http_code"], " ", $i["content_type"], " ", $i["size_download"], " ", $i["download_content_length"], " ", $i["redirect_count"], " ", $i["scheme"], " ", $i["effective_method"], " ", $i["http_version"], " ", $i["protocol"], "\n";
var_dump(curl_errno($ch), curl_error($ch));

echo "--- options\n";
curl_setopt_array($ch, [
    CURLOPT_USERAGENT => "p8-test/1.0",
    CURLOPT_REFERER => "http://ref.example/",
    CURLOPT_COOKIE => "a=1; b=2",
    CURLOPT_HTTPHEADER => ["X-One: 1", "X-Two:two", "Accept: text/plain", "X-Empty;"],
    CURLOPT_URL => "$base/echo",
]);
echo curl_exec($ch);

echo "--- remove default header\n";
curl_setopt($ch, CURLOPT_HTTPHEADER, ["Accept:"]);
echo curl_exec($ch);

echo "--- header out\n";
curl_setopt($ch, CURLINFO_HEADER_OUT, true);
curl_setopt($ch, CURLOPT_HTTPHEADER, ["X-One: 1"]);
curl_exec($ch);
echo clean(curl_getinfo($ch, CURLINFO_HEADER_OUT));
echo clean(curl_getinfo($ch)["request_header"]);

echo "--- header included\n";
$ch = curl_init("$base/headers");
curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_HEADER => true]);
$r = curl_exec($ch);
echo $r;
var_dump(curl_getinfo($ch, CURLINFO_HEADER_SIZE), strlen($r) - curl_getinfo($ch, CURLINFO_HEADER_SIZE), curl_getinfo($ch, CURLINFO_CONTENT_TYPE));

echo "--- nobody\n";
$ch = curl_init("$base/headers");
curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_NOBODY => true, CURLOPT_HEADER => true]);
echo curl_exec($ch);
var_dump(curl_getinfo($ch, CURLINFO_EFFECTIVE_METHOD), curl_getinfo($ch, CURLINFO_SIZE_DOWNLOAD));

echo "--- custom request\n";
foreach (["DELETE", "PATCH", "OPTIONS"] as $m) {
    $ch = curl_init("$base/echo");
    curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_CUSTOMREQUEST => $m]);
    echo strtok(curl_exec($ch), "\n"), "\n";
}

echo "--- basic auth\n";
$ch = curl_init("$base/auth");
curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
var_dump(curl_exec($ch), curl_getinfo($ch, CURLINFO_HTTP_CODE));
curl_setopt($ch, CURLOPT_USERPWD, "user:pa ss");
var_dump(curl_exec($ch), curl_getinfo($ch, CURLINFO_HTTP_CODE));
curl_setopt($ch, CURLOPT_USERPWD, null);
curl_setopt($ch, CURLOPT_USERNAME, "u");
curl_setopt($ch, CURLOPT_PASSWORD, "p");
var_dump(curl_exec($ch));

echo "--- status and failonerror\n";
$ch = curl_init("$base/status/404");
curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
var_dump(curl_exec($ch), curl_getinfo($ch, CURLINFO_HTTP_CODE), curl_errno($ch));
curl_setopt($ch, CURLOPT_FAILONERROR, true);
var_dump(curl_exec($ch), curl_getinfo($ch, CURLINFO_HTTP_CODE), curl_errno($ch), curl_error($ch));

echo "--- chunked and big\n";
$ch = curl_init("$base/chunked");
curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
var_dump(curl_exec($ch));
$ch = curl_init("$base/big");
curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
$big = curl_exec($ch);
var_dump(strlen($big), md5($big), curl_getinfo($ch, CURLINFO_SIZE_DOWNLOAD));

echo "--- encoding\n";
$ch = curl_init("$base/gzip");
curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
var_dump(curl_exec($ch));
curl_setopt($ch, CURLOPT_ENCODING, "");
var_dump(curl_exec($ch));
curl_setopt($ch, CURLOPT_ENCODING, "gzip");
var_dump(curl_exec($ch));

echo "--- cookie engine\n";
$ch = curl_init("$base/cookie-set");
curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_COOKIEFILE => ""]);
curl_exec($ch);
curl_setopt($ch, CURLOPT_URL, "$base/cookie-show");
$r = curl_exec($ch);
echo preg_match('/^cookie: (.*)$/m', $r, $m) ? implode("; ", (function ($c) { sort($c); return $c; })(explode("; ", $m[1]))) : "none", "\n";
var_dump(count(curl_getinfo($ch, CURLINFO_COOKIELIST)));

echo "--- output to a file\n";
$ch = curl_init("$base/headers");
$fp = fopen("php://memory", "w+");
$hp = fopen("php://memory", "w+");
curl_setopt_array($ch, [CURLOPT_FILE => $fp, CURLOPT_WRITEHEADER => $hp]);
var_dump(curl_exec($ch));
rewind($fp); rewind($hp);
var_dump(stream_get_contents($fp));
echo stream_get_contents($hp);

echo "--- stdout\n";
$ch = curl_init("$base/headers");
var_dump(curl_exec($ch));
