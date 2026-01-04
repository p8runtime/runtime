<?php
$value = 2;
if ($value === 1) {
    echo "one\n";
} elseif ($value === 2) {
    echo "two\n";
} else {
    echo "other\n";
}

$value = 3;
if ($value < 2) {
    echo "small\n";
} elseif ($value < 5) {
    echo "medium\n";
} elseif ($value < 10) {
    echo "large\n";
} else {
    echo "huge\n";
}
