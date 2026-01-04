<?php
// https://www.php.net/manual/en/functions.user-defined.php
function greet(string $name): string {
    return "Hello, $name!";
}
echo greet("Gild") . "\n";
