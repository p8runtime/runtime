<?php
$a = 5;
$b = 2;
$sum = $a + $b;
$diff = $a - $b;
$bitmask = $a & $b;
$combined = "$a$b";
$maybe = null;
$coalesced = $maybe ?? 'fallback';
$spaceship = $a <=> $b;

echo "sum: $sum\n";
echo "diff: $diff\n";
echo "bitmask: $bitmask\n";
echo "combined: $combined\n";
echo "fallback: $coalesced\n";
echo "spaceship: $spaceship\n";
