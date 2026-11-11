<?php
// Included by index.php.

function page_title(string $page): string
{
    return 'Page: ' . htmlspecialchars($page);
}

function greeting(): string
{
    return 'Hello from p8 (PHP ' . PHP_MAJOR_VERSION . '.' . PHP_MINOR_VERSION . ')';
}
