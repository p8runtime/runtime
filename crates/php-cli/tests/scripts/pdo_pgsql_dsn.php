<?php
// pdo_pgsql without a server: what connecting reports for DSNs libpq
// refuses or cannot reach (no server listens on these).
$dsns = [
    'pgsql:nosuchoption=1',
    'pgsql:host',
    "pgsql:dbname='unterminated",
    'pgsql:host=localhost;sslmode=sometimes',
    'pgsql:host=127.0.0.1;port=1;dbname=x',
    'pgsql:host=/nonexistent/dir;dbname=x',
    'pgsql:host=127.0.0.1;port=notaport',
];
foreach ($dsns as $dsn) {
    try {
        new PDO($dsn, 'u', 'p');
        echo "connected?\n";
    } catch (PDOException $e) {
        echo $dsn, "\n  ", get_class($e), ' ', var_export($e->getCode(), true), "\n  ", $e->getMessage(), "\n";
        var_dump($e->errorInfo);
    }
}
try {
    Pdo\Pgsql::connect('sqlite::memory:');
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
try {
    new Pdo\Sqlite('pgsql:host=/nonexistent');
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
try {
    new Pdo\Pgsql('pgsql:host=/nonexistent/dir');
} catch (PDOException $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
