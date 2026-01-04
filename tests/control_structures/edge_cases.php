<?php
$x = 2;
switch ($x) {
    case 1:
        echo "one";
        break;
    case 2:
        echo "two";
    case 3:
        echo "three";
        break;
    default:
        echo "other";
}

echo "\n";

$sum = 0;
for ($i = 0; $i < 5; $i++) {
    if ($i === 3) {
        continue;
    }
    $sum += $i;
}
echo $sum . "\n";

$i = 0;
do {
    $i++;
} while ($i < 2);
echo $i . "\n";
