<?php
// https://www.php.net/manual/en/ref.strings.php
$text = "PHP and Gild";
echo "Original: $text\n";
echo "Substring: " . substr($text, 0, 3) . "\n";
echo "Replace: " . str_replace("Gild", "GPT", $text) . "\n";
