<?php
// Float formatting: precision=14 for echo/print_r/string casts, shortest
// round-trip (serialize_precision=-1) for var_dump/var_export/json/serialize.
foreach ([1.0, -0.0, 0.1 + 0.2, 1 / 3, 1e14, 1e15, 123456789012345.0, 1234567890123456.0, 1e16, 1.5e17, 1e25, 0.0001, 0.00001, 1.5e-7, -2.5e-5, 123.456, 1e100, 9007199254740993.0, 1e-10, 0.1, NAN, INF, -INF] as $f) {
    echo serialize($f), " | ";
    var_dump($f);
    echo " | ", var_export($f, true), " | ", @json_encode($f), " | ", $f, " | ", print_r($f, true), " | ", "x$f", " | ", implode(',', [$f]), "\n";
}
echo json_encode(1.0, JSON_PRESERVE_ZERO_FRACTION), "\n";
