<?php
// curl_setopt validation, handle classes, errors and getinfo without a server.

function show(callable $f) {
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

$ch = curl_init();
var_dump($ch instanceof CurlHandle);
show(fn() => new CurlHandle());
show(fn() => clone $ch instanceof CurlHandle);
show(fn() => serialize($ch));
show(fn() => curl_setopt($ch, -10, 0));
show(fn() => curl_setopt($ch, 1000, 0));
show(fn() => curl_setopt($ch, CURLOPT_URL, "http://a\0b"));
show(fn() => curl_setopt($ch, CURLOPT_HTTPHEADER, "x"));
show(fn() => curl_setopt($ch, CURLOPT_HTTPHEADER, ["A: b", 5]));
show(fn() => curl_setopt($ch, CURLOPT_WRITEFUNCTION, "nope"));
show(fn() => curl_setopt($ch, CURLOPT_HEADERFUNCTION, [new stdClass, "m"]));
show(fn() => curl_setopt($ch, CURLOPT_SAFE_UPLOAD, false));
show(fn() => curl_setopt($ch, CURLOPT_FILE, "str"));
show(fn() => curl_setopt($ch, CURLOPT_FILE, fopen("php://memory", "r")));
show(fn() => curl_setopt($ch, CURLOPT_FILE, fopen("php://memory", "w")));
show(fn() => curl_setopt($ch, CURLOPT_FILE, null));
show(fn() => curl_setopt($ch, CURLOPT_RETURNTRANSFER, true));
show(fn() => curl_setopt($ch, CURLOPT_SSL_VERIFYHOST, 1));
show(fn() => curl_setopt($ch, CURLOPT_PRIVATE, ["x" => 1]));
show(fn() => curl_getinfo($ch, CURLINFO_PRIVATE));
show(fn() => curl_setopt_array($ch, [CURLOPT_TIMEOUT => 5, CURLOPT_USERAGENT => "x"]));
show(fn() => curl_setopt_array($ch, [CURLOPT_TIMEOUT => 5, 1000 => 1]));
show(fn() => curl_setopt_array($ch, ["a" => 1]));
show(fn() => curl_setopt($ch, CURLINFO_HEADER_OUT, true));
show(fn() => curl_setopt($ch, CURLOPT_DEBUGFUNCTION, fn() => 0));
show(fn() => curl_setopt($ch, CURLINFO_HEADER_OUT, true));

// Errors without a transfer.
$ch = curl_init();
var_dump(curl_exec($ch), curl_errno($ch), curl_error($ch));
$ch = curl_init("nosuchscheme://x");
var_dump(curl_exec($ch), curl_errno($ch), curl_error($ch));
$ch = curl_init("http://x.invalid/");
curl_setopt($ch, CURLOPT_PROXY, "");
var_dump(curl_errno($ch), curl_error($ch));

// getinfo of a handle that never ran.
$ch = curl_init("http://example.com/");
$info = curl_getinfo($ch);
echo implode(",", array_keys($info)), "\n";
var_dump($info["url"], $info["http_code"], $info["content_type"], $info["download_content_length"], $info["primary_port"], $info["scheme"]);
var_dump(curl_getinfo($ch, CURLINFO_HTTP_CODE), curl_getinfo($ch, CURLINFO_EFFECTIVE_URL), curl_getinfo($ch, CURLINFO_CONTENT_TYPE), curl_getinfo($ch, CURLINFO_HEADER_OUT));

// reset, copy, close
$ch = curl_init("http://example.com/");
curl_setopt($ch, CURLOPT_PRIVATE, "p");
$copy = curl_copy_handle($ch);
var_dump(curl_getinfo($copy, CURLINFO_PRIVATE), curl_getinfo($copy, CURLINFO_EFFECTIVE_URL));
curl_reset($ch);
var_dump(curl_getinfo($ch, CURLINFO_EFFECTIVE_URL), curl_getinfo($copy, CURLINFO_EFFECTIVE_URL));
var_dump(curl_close($ch));

// strerror, escape
var_dump(curl_strerror(0), curl_strerror(6), curl_strerror(28), curl_strerror(999));
var_dump(curl_multi_strerror(0), curl_multi_strerror(7), curl_share_strerror(1), curl_share_strerror(99));
$ch = curl_init();
var_dump(curl_escape($ch, "a b&c/d~e-f_g.h é"), curl_unescape($ch, "a%20b%26c%2Fd%zz%4"));

// CURLFile & friends
$f = new CURLFile("/etc/hosts", "text/plain", "h.txt");
var_dump($f->getFilename(), $f->getMimeType(), $f->getPostFilename());
$f->setMimeType("a/b"); $f->setPostFilename("p");
var_dump($f->getMimeType(), $f->getPostFilename());
$g = curl_file_create("x");
var_dump($g->name, $g->mime, $g->postname);
$s = new CURLStringFile("data", "d.txt");
var_dump($s->data, $s->postname, $s->mime);
show(fn() => serialize($f));
show(fn() => unserialize('O:8:"CURLFile":0:{}'));

// constants
var_dump(CURLOPT_URL, CURLOPT_RETURNTRANSFER, CURLINFO_HTTP_CODE, CURLE_OPERATION_TIMEDOUT, CURLE_COULDNT_RESOLVE_HOST, CURL_HTTP_VERSION_1_1, CURLM_OK, CURLMSG_DONE, CURLAUTH_BASIC, CURLPROTO_HTTPS);
$v = curl_version();
var_dump(is_string($v["version"]), is_int($v["version_number"]), in_array("https", $v["protocols"]), $v["feature_list"]["SSL"]);

// multi handles
$mh = curl_multi_init();
var_dump($mh instanceof CurlMultiHandle);
show(fn() => new CurlMultiHandle());
show(fn() => clone $mh);
show(fn() => curl_multi_setopt($mh, 99999, 1));
show(fn() => curl_multi_setopt($mh, CURLMOPT_MAXCONNECTS, 5));
show(fn() => curl_multi_select($mh, -1));
$a = curl_init(); $b = curl_init();
var_dump(curl_multi_add_handle($mh, $a), curl_multi_add_handle($mh, $a), curl_multi_remove_handle($mh, $a), curl_multi_info_read($mh), curl_multi_errno($mh));
$sh = curl_share_init();
var_dump($sh instanceof CurlShareHandle, curl_share_setopt($sh, CURLSHOPT_SHARE, CURL_LOCK_DATA_COOKIE), curl_share_errno($sh), curl_share_setopt($sh, CURLSHOPT_SHARE, 99), curl_share_errno($sh));
show(fn() => curl_share_setopt($sh, 99999, 1));

// file:// transfers
$path = tempnam(sys_get_temp_dir(), "curl");
file_put_contents($path, "file body\n");
$ch = curl_init("file://" . $path);
curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
var_dump(curl_exec($ch), curl_getinfo($ch, CURLINFO_SIZE_DOWNLOAD), curl_errno($ch));
$ch = curl_init("file://" . $path);
var_dump(curl_exec($ch));
$ch = curl_init("file://" . $path . ".missing");
var_dump(curl_exec($ch), curl_errno($ch));
unlink($path);
