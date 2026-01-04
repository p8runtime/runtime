<?php
// https://www.php.net/manual/en/language.types.array.php
$fruits = ["apple", "banana", "cherry"];
echo "Fruit list:\n";
foreach ($fruits as $index => $fruit) {
    echo ($index + 1) . ". $fruit\n";
}
echo "Uppercase fruit names:\n";
function uppercase(string $fruit): string {
    return strtoupper($fruit);
}
$upper = [];
foreach ($fruits as $fruit) {
    $upper[] = uppercase($fruit);
}
print_r($upper);
