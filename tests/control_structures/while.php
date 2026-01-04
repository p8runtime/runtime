<?php
$count = 0;
while ($count < 3) {
    echo "while $count\n";
    $count++;
}

$value = 5;
while ($value > 0):
    echo "value $value\n";
    $value -= 2;
endwhile;
