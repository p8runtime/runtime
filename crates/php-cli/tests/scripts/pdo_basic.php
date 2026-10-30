<?php
var_dump(extension_loaded('pdo'), extension_loaded('pdo_sqlite'));
var_dump(in_array('sqlite', PDO::getAvailableDrivers()), pdo_drivers());

$db = new PDO('sqlite::memory:');
var_dump(get_class($db), $db->getAttribute(PDO::ATTR_DRIVER_NAME));
var_dump($db->getAttribute(PDO::ATTR_ERRMODE) === PDO::ERRMODE_EXCEPTION);
var_dump($db->errorCode());

var_dump($db->exec('CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, score REAL, data BLOB)'));
var_dump($db->exec("INSERT INTO t (name, score) VALUES ('a', 1.5), ('b', 2.5), ('c', NULL)"));
var_dump($db->lastInsertId());
var_dump($db->errorCode());
var_dump($db->errorInfo());

// positional placeholders
$st = $db->prepare('SELECT * FROM t WHERE id > ? AND name <> ?');
var_dump($st instanceof PDOStatement, $st->queryString);
var_dump($st->execute([1, 'x']));
var_dump($st->columnCount(), $st->rowCount());
var_dump($st->fetchAll(PDO::FETCH_ASSOC));

// named placeholders
$st = $db->prepare('SELECT name FROM t WHERE id = :id');
$st->execute([':id' => 2]);
var_dump($st->fetchColumn());
$st->execute(['id' => 3]);
var_dump($st->fetchColumn());

// placeholders inside strings and comments are not placeholders
$st = $db->prepare("SELECT '?' AS q, ':x' AS n, ? AS p -- :ignored ?\n");
$st->execute(['v']);
var_dump($st->fetch(PDO::FETCH_ASSOC));

// insert through a prepared statement
$st = $db->prepare('INSERT INTO t (name, score) VALUES (:n, :s)');
var_dump($st->execute([':n' => 'd', ':s' => 4.25]));
var_dump($st->rowCount(), $db->lastInsertId());

// query() with a fetch mode
foreach ($db->query('SELECT id, name FROM t ORDER BY id', PDO::FETCH_NUM) as $k => $row) {
    echo $k, ': ', implode(',', $row), "\n";
}
var_dump($db->query('SELECT COUNT(*) FROM t')->fetchColumn());

// quote
var_dump($db->quote("it's"), $db->quote('plain'));

// column meta
$st = $db->query('SELECT id, name, score FROM t WHERE id = 1');
var_dump($st->getColumnMeta(0));
var_dump($st->getColumnMeta(2)['native_type']);
var_dump($st->getColumnMeta(1)['sqlite:decl_type']);

// driver names
var_dump($db->getAttribute(PDO::ATTR_SERVER_VERSION) === $db->getAttribute(PDO::ATTR_CLIENT_VERSION));
$d2 = PDO::connect('sqlite::memory:');
var_dump(get_class($d2));
var_dump($d2 instanceof PDO);
