<?php
$f = tmpfile(); fwrite($f, "hello"); $m = stream_get_meta_data($f);
var_dump($m["uri"] !== "php://temp", is_file($m["uri"]), file_get_contents($m["uri"]), $m["wrapper_type"], $m["stream_type"], $m["mode"]);
rewind($f); var_dump(fread($f, 10));
$p = $m["uri"]; fclose($f); var_dump(file_exists($p));
$g = tmpfile(); $q = stream_get_meta_data($g)["uri"]; unset($g); var_dump(file_exists($q));
