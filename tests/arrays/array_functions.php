<?php
$colors = ["red", "green", "blue"];
$more = ["yellow", "purple"];
$merged = array_merge($colors, $more);
echo "Merged colors: " . implode(", ", $merged) . "\n";
$filtered = [];
foreach ($merged as $color) {
    if (strlen($color) > 4) {
        $filtered[] = $color;
    }
}
echo "Filtered (length>4): " . implode(", ", $filtered) . "\n";
