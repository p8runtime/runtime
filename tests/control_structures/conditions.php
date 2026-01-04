<?php
// https://www.php.net/manual/en/control-structures.ifs.php
$temperature = 72;
if ($temperature > 85) {
    echo "It's hot outside.\n";
} elseif ($temperature >= 60) {
    echo "It's mild.\n";
} else {
    echo "It's chilly.\n";
}
