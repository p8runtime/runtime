<?php
// https://www.php.net/manual/en/language.exceptions.php
function mayThrow(bool $error): string {
    if ($error) {
        throw new Exception("Manual exception");
    }
    return "Success";
}

try {
    echo mayThrow(false) . "\n";
    echo mayThrow(true) . "\n";
} catch (Exception $e) {
    echo "Caught: " . $e->getMessage() . "\n";
}
