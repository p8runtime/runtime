<?php
$value = "42";
$intValue = (int) $value;
$floatValue = (float) $value;
$boolValue = (bool) $value;

echo "int: $intValue\n";
echo "float: $floatValue\n";
echo "bool: " . ($boolValue ? 'true' : 'false') . "\n";
