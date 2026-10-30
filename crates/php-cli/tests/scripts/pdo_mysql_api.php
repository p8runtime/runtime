<?php
// pdo_mysql's API, without a server: the extension, the driver, PDO's
// MYSQL_ATTR_* constants, the Pdo\Mysql subclass, and how a connection
// that cannot be made fails.
var_dump(extension_loaded('pdo_mysql'));
var_dump(PDO::getAvailableDrivers(), pdo_drivers());
foreach ((new ReflectionClass(PDO::class))->getConstants() as $name => $value) {
    if (str_starts_with($name, 'MYSQL_')) {
        echo "PDO::$name = $value\n";
    }
}
$r = new ReflectionClass(Pdo\Mysql::class);
echo $r->getName(), ' extends ', $r->getParentClass()->getName(), "\n";
foreach ($r->getConstants() as $name => $value) {
    if ($r->getReflectionConstant($name)->getDeclaringClass()->getName() === Pdo\Mysql::class) {
        echo "Pdo\\Mysql::$name = $value\n";
    }
}
foreach ($r->getMethods() as $m) {
    if ($m->class === Pdo\Mysql::class) {
        echo $m->getName(), "\n";
    }
}
var_dump(Pdo\Mysql::ATTR_USE_BUFFERED_QUERY === PDO::MYSQL_ATTR_USE_BUFFERED_QUERY);

// Nothing listens on port 1; a socket that does not exist.
foreach (['mysql:host=127.0.0.1;port=1;dbname=x', 'mysql:host=localhost;unix_socket=/nonexistent/p8.sock', 'mysql:host=127.0.0.1;port=1;charset=nonesuch'] as $dsn) {
    try {
        new PDO($dsn, 'u', 'p');
    } catch (PDOException $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
        var_dump($e->getCode(), $e->errorInfo);
    }
}
try {
    Pdo\Mysql::connect('mysql:host=127.0.0.1;port=1', 'u', 'p');
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
try {
    Pdo\Mysql::connect('sqlite::memory:');
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
var_dump(get_class(PDO::connect('sqlite::memory:')));
try {
    (new ReflectionClass(Pdo\Mysql::class))->newInstanceWithoutConstructor()->getWarningCount();
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
