<?php
$db = new PDO('sqlite::memory:');
$db->exec('CREATE TABLE a (id INTEGER PRIMARY KEY, v TEXT)');
function count_rows($db) { return (int) $db->query('SELECT COUNT(*) FROM a')->fetchColumn(); }

var_dump($db->inTransaction());
var_dump($db->beginTransaction(), $db->inTransaction());
$db->exec("INSERT INTO a (v) VALUES ('one')");
var_dump(count_rows($db));
var_dump($db->rollBack(), $db->inTransaction());
var_dump(count_rows($db));

$db->beginTransaction();
$db->exec("INSERT INTO a (v) VALUES ('two')");
$st = $db->prepare('INSERT INTO a (v) VALUES (?)');
$st->execute(['three']);
var_dump($db->commit(), $db->inTransaction());
var_dump(count_rows($db));

// a failed statement inside a transaction leaves the transaction open
$db->beginTransaction();
try {
    $db->exec("INSERT INTO a (id, v) VALUES (1, 'dup')");
} catch (PDOException $e) {
    echo $e->getCode(), "\n";
}
var_dump($db->inTransaction());
$db->rollBack();

// explicit SQL transaction control is visible to inTransaction()
$db->exec('BEGIN');
var_dump($db->inTransaction());
$db->exec('COMMIT');
var_dump($db->inTransaction());

// lastInsertId follows the connection
$db->exec("INSERT INTO a (v) VALUES ('four')");
var_dump($db->lastInsertId());
$st = $db->prepare('INSERT INTO a (id, v) VALUES (100, ?)');
$st->execute(['x']);
var_dump($db->lastInsertId(), $db->lastInsertId('ignored'));

// rowCount for each statement kind
$st = $db->prepare('UPDATE a SET v = ? WHERE id < 100');
$st->execute(['u']);
var_dump($st->rowCount());
$st = $db->prepare('DELETE FROM a WHERE id = 100');
$st->execute();
var_dump($st->rowCount());
var_dump($db->exec("UPDATE a SET v = 'w'"));
var_dump($db->exec("UPDATE a SET v = 'w' WHERE id = -1"));
