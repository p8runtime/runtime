<?php
$value = 3;
switch ($value) {
    case 1:
        echo "one\n";
        break;
    case 2:
    case 3:
        echo "two or three\n";
        break;
    default:
        echo "other\n";
}
