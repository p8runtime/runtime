<?php
// https://www.php.net/manual/en/control-structures.for.php
for ($i = 1; $i <= 3; $i++) {
    echo "for loop iteration: $i\n";
}

$count = 3;
while ($count > 0) {
    echo "while: $count\n";
    $count--;
}

$items = ["red", "green", "blue"];
foreach ($items as $index => $color) {
    echo ($index + 1) . ": $color\n";
}
