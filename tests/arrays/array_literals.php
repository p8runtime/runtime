<?php
// https://www.php.net/manual/en/language.types.array.php
$colors = ["red", "green", "blue"];
array_push($colors, "yellow");
echo "Colors: " . implode(", ", $colors) . "\n";

$lookup = ["PHP" => "language", "Gild" => "runtime"];
foreach ($lookup as $key => $value) {
    echo "$key => $value\n";
}
