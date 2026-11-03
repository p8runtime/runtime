<?php
$p = proc_open(['sh', '-c', 'sleep 0.3; echo out; echo err >&2'], [1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes);
stream_set_blocking($pipes[1], false);
stream_set_blocking($pipes[2], false);
$r = [$pipes[1], $pipes[2]]; $w = null; $e = null;
var_dump(stream_select($r, $w, $e, 0, 0), $r);
$got = ['', ''];
$open = [1 => $pipes[1], 2 => $pipes[2]];
while ($open) {
    $r = $open; $w = []; $e = [];
    if (stream_select($r, $w, $e, 5) === false) { break; }
    foreach ($r as $k => $pipe) {
        $chunk = fread($pipe, 8192);
        if ($chunk === '' && feof($pipe)) { unset($open[$k]); continue; }
        $got[$k - 1] .= $chunk;
    }
}
var_dump($got, $e);
var_dump(proc_close($p));
$w = [STDOUT];
$r = null;
var_dump(stream_select($r, $w, $e, 0));
try { $n = null; stream_select($n, $n, $n, 0); } catch (ValueError $x) { echo $x->getMessage(), "\n"; }
