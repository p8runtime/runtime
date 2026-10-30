<?php
$cases = [
    ['*.txt', 'a.txt', 0], ['*.txt', 'dir/a.txt', 0], ['*.txt', 'dir/a.txt', FNM_PATHNAME],
    ['*/*.txt', 'dir/a.txt', FNM_PATHNAME], ['*', '.hidden', 0], ['*', '.hidden', FNM_PERIOD],
    ['.*', '.hidden', FNM_PERIOD], ['dir/*', 'dir/.h', FNM_PATHNAME | FNM_PERIOD], ['dir/*', 'dir/.h', FNM_PERIOD],
    ['A?C', 'abc', FNM_CASEFOLD], ['A?C', 'abc', 0], ['[a-c]x', 'bx', 0], ['[!a-c]x', 'bx', 0], ['[]]', ']', 0],
    ['\*', '*', 0], ['\*', '\x', FNM_NOESCAPE], ['\*', '\*', FNM_NOESCAPE], ['a\\b', 'ab', 0],
    ['192.168.*', '192.168.1.1', 0], ['*.example.com', 'www.example.com', 0], ['?', '/', FNM_PATHNAME], ['[/]', '/', FNM_PATHNAME],
    ['', '', 0], ['*', '', 0], ['a*b*c', 'axxbyyc', 0], ['a*b*c', 'axxbyy', 0],
];
foreach ($cases as [$p, $n, $f]) { echo str_pad("$p | $n | $f", 30), var_export(fnmatch($p, $n, $f), true), "\n"; }
