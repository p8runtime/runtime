<?php
// Errors: MySQL's SQLSTATEs, codes and messages through errorCode(),
// errorInfo() and the three error modes; transactions.
$db = new PDO('mysql:host=' . getenv('DBHOST') . ';port=' . getenv('DBPORT') . ';dbname=pdo_p8', 'wp', 'wp');
$db->exec('DROP TABLE IF EXISTS errors_t');
$db->exec('CREATE TABLE errors_t (id INT PRIMARY KEY, v VARCHAR(5) NOT NULL) ENGINE=InnoDB');
$db->exec("INSERT INTO errors_t VALUES (1, 'one')");

// Objects by their class (handle numbers are the engine's business).
function show($v): void {
    if (is_object($v)) {
        echo 'object(', get_class($v), ")\n";
    } else {
        var_dump($v);
    }
}

function attempt(callable $f): void {
    try {
        show($f());
    } catch (PDOException $e) {
        echo 'PDOException: ', $e->getMessage(), "\n";
        var_dump($e->getCode(), $e->errorInfo);
    }
}

var_dump($db->errorCode(), $db->errorInfo());
foreach ([true, false] as $emulate) {
    echo "--- emulate=", var_export($emulate, true), "\n";
    $db->setAttribute(PDO::ATTR_EMULATE_PREPARES, $emulate);
    attempt(fn () => $db->query('SELECT * FROM no_such_table'));
    attempt(fn () => $db->exec('SELEC 1'));
    attempt(fn () => $db->exec("INSERT INTO errors_t VALUES (1, 'dup')"));
    attempt(fn () => $db->exec('INSERT INTO errors_t (id) VALUES (2)'));
    attempt(fn () => $db->prepare('SELECT nope FROM errors_t'));
    $st = $db->prepare('INSERT INTO errors_t VALUES (?, ?)');
    attempt(fn () => $st->execute([1, 'x']));
    var_dump($st->errorCode(), $st->errorInfo());
    attempt(fn () => $st->execute([3, null]));
    attempt(fn () => $st->execute([4, 'toolong']));
    var_dump($db->errorCode());

    $db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_SILENT);
    var_dump($db->query('SELECT * FROM no_such_table'), $db->errorCode(), $db->errorInfo());
    var_dump($db->exec('SELEC 1'), $db->errorInfo());
    $st = $db->prepare('SELECT * FROM errors_t WHERE id = ?');
    var_dump($st->execute([1]), $st->errorInfo(), $db->errorInfo());
    $st = $db->prepare('SELECT nope FROM errors_t');
    show($st);
    var_dump($db->errorInfo());

    $db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_WARNING);
    var_dump($db->exec("INSERT INTO errors_t VALUES (1, 'dup')"));
    $st = $db->prepare('INSERT INTO errors_t VALUES (?, ?)');
    var_dump($st->execute([1, 'x']));
    $db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION);
}

// Transactions.
$db->setAttribute(PDO::ATTR_EMULATE_PREPARES, true);
var_dump($db->inTransaction(), $db->beginTransaction(), $db->inTransaction());
$db->exec("INSERT INTO errors_t VALUES (10, 'ten')");
var_dump($db->rollBack(), $db->inTransaction());
var_dump($db->query('SELECT COUNT(*) FROM errors_t WHERE id = 10')->fetchColumn());
$db->beginTransaction();
$db->exec("INSERT INTO errors_t VALUES (11, 'elf')");
var_dump($db->commit());
var_dump($db->query('SELECT COUNT(*) FROM errors_t WHERE id = 11')->fetchColumn());
attempt(fn () => $db->commit());
attempt(fn () => $db->rollBack());
$db->beginTransaction();
attempt(fn () => $db->beginTransaction());
// DDL commits implicitly: the server says the transaction is over.
$db->exec('CREATE TABLE IF NOT EXISTS errors_t2 (id INT)');
var_dump($db->inTransaction());
attempt(fn () => $db->commit());
$db->exec('DROP TABLE errors_t2');
// A transaction started in SQL is seen too.
$db->exec('START TRANSACTION');
var_dump($db->inTransaction());
$db->exec('COMMIT');
var_dump($db->inTransaction());
// Autocommit off: every statement is in a transaction until it ends.
$db->setAttribute(PDO::ATTR_AUTOCOMMIT, false);
$db->exec("INSERT INTO errors_t VALUES (12, 'tw')");
var_dump($db->inTransaction(), $db->query('SELECT @@autocommit')->fetchColumn());
attempt(fn () => $db->beginTransaction());
$db->exec('ROLLBACK');
$db->setAttribute(PDO::ATTR_AUTOCOMMIT, true);
var_dump($db->query('SELECT COUNT(*) FROM errors_t WHERE id = 12')->fetchColumn());

// A lost connection.
$k = new PDO('mysql:host=' . getenv('DBHOST') . ';port=' . getenv('DBPORT') . ';dbname=pdo_p8', 'wp', 'wp');
$id = $k->query('SELECT CONNECTION_ID()')->fetchColumn();
$db->exec("KILL $id");
attempt(fn () => $k->query('SELECT 1'));
attempt(fn () => $k->query('SELECT 1'));
$db->exec('DROP TABLE errors_t');

// exec() of a query that returns rows leaves them unread: the connection
// is out of sync from then on.
$s = new PDO('mysql:host=' . getenv('DBHOST') . ';port=' . getenv('DBPORT') . ';dbname=pdo_p8', 'wp', 'wp');
var_dump($s->exec('SELECT 1'), $s->errorInfo());
attempt(fn () => $s->query('SELECT 2'));
attempt(fn () => $s->prepare('SELECT 3'));
$s->setAttribute(PDO::ATTR_EMULATE_PREPARES, false);
attempt(fn () => $s->prepare('SELECT 4'));
attempt(fn () => $s->beginTransaction());
attempt(fn () => $s->getAttribute(PDO::ATTR_SERVER_INFO));
attempt(fn () => $s->setAttribute(PDO::ATTR_AUTOCOMMIT, false));
var_dump($s->errorInfo());

// After a failed query() the connection reports its error, thrown or not.
$e = new PDO('mysql:host=' . getenv('DBHOST') . ';port=' . getenv('DBPORT') . ';dbname=pdo_p8', 'wp', 'wp');
attempt(fn () => $e->query('SELEC 1'));
var_dump($e->errorCode(), $e->errorInfo());
// An error PDO raises itself shows the connection's last driver error too.
$e->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_WARNING);
var_dump($e->getAttribute(-1), $e->errorInfo());
$e->setAttribute(PDO::ATTR_EMULATE_PREPARES, false);
var_dump($e->prepare('SELECT ?, :a'));
// INIT_COMMAND may hold several statements.
$i = new PDO('mysql:host=' . getenv('DBHOST') . ';port=' . getenv('DBPORT') . ';dbname=pdo_p8', 'wp', 'wp', [PDO::MYSQL_ATTR_INIT_COMMAND => 'SET @a = 1; SET @b = 2', PDO::MYSQL_ATTR_LOCAL_INFILE_DIRECTORY => '']);
var_dump($i->query('SELECT @a + @b')->fetchColumn(), $i->getAttribute(PDO::MYSQL_ATTR_LOCAL_INFILE_DIRECTORY));
