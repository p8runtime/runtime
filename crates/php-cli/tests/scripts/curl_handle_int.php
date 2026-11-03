<?php
// (int) of a curl handle is its object id (curl_cast_object), the number
// code keys its transfers by (Composer's CurlDownloader); two handles differ.
$a = curl_init();
$b = curl_init();
$m = curl_multi_init();
var_dump((int) $a === spl_object_id($a), (int) $b === spl_object_id($b), (int) $a !== (int) $b, (int) $m === spl_object_id($m));
$jobs = [(int) $a => "a", (int) $b => "b"];
curl_multi_add_handle($m, $a);
curl_multi_add_handle($m, $b);
var_dump(count($jobs), $jobs[(int) $b]);
