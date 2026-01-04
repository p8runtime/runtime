<?php
function factorial(int $n): int
{
    return $n <= 1 ? 1 : $n * factorial($n - 1);
}
foreach ([0, 1, 5] as $value) {
    echo "factorial({$value}) = " . factorial($value) . "\n";
}
