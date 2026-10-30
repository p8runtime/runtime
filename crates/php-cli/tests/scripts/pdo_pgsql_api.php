<?php
// pdo_pgsql's API, without a server: the extension, the driver, PDO's
// PGSQL_* constants and the Pdo\Pgsql subclass.
var_dump(extension_loaded('pdo_pgsql'));
var_dump(in_array('pgsql', PDO::getAvailableDrivers(), true));
var_dump(in_array('pgsql', pdo_drivers(), true));
foreach (['PGSQL_ATTR_DISABLE_PREPARES', 'PGSQL_TRANSACTION_IDLE', 'PGSQL_TRANSACTION_ACTIVE', 'PGSQL_TRANSACTION_INTRANS', 'PGSQL_TRANSACTION_INERROR', 'PGSQL_TRANSACTION_UNKNOWN'] as $c) {
    echo "PDO::$c = ", constant("PDO::$c"), "\n";
}
$r = new ReflectionClass(Pdo\Pgsql::class);
echo $r->getName(), ' extends ', $r->getParentClass()->getName(), "\n";
foreach ($r->getConstants() as $name => $value) {
    if ($r->getReflectionConstant($name)->getDeclaringClass()->getName() === Pdo\Pgsql::class) {
        echo "Pdo\\Pgsql::$name = $value\n";
    }
}
foreach ($r->getMethods() as $m) {
    if ($m->class === Pdo\Pgsql::class) {
        echo $m->getName(), "\n";
    }
}
var_dump(Pdo\Pgsql::ATTR_DISABLE_PREPARES === PDO::PGSQL_ATTR_DISABLE_PREPARES);
var_dump(is_subclass_of(Pdo\Pgsql::class, PDO::class));
// A configuration that reads the attribute (as Coolify's does).
$options = [PDO::PGSQL_ATTR_DISABLE_PREPARES => true, PDO::ATTR_EMULATE_PREPARES => false];
var_dump($options);
try {
    (new ReflectionClass(Pdo\Pgsql::class))->newInstanceWithoutConstructor()->getPid();
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
