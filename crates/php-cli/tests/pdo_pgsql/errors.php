<?php
// Errors, transactions and the Pdo\Pgsql methods against PostgreSQL.
$db = Pdo\Pgsql::connect('pgsql:host=' . getenv('DBHOST') . ';port=' . getenv('DBPORT') . ';dbname=test', 'postgres', 'p8');
echo get_class($db), "\n";

try {
    $db->query('SELECT * FROM no_such_table');
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
    var_dump($e->getCode(), $e->errorInfo);
}
try {
    $db->exec("SELEC 1");
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
$st = $db->prepare('SELECT 1/0');
try {
    $st->execute();
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
    var_dump($st->errorCode(), $st->errorInfo());
}
$st = $db->prepare('SELECT :a');
try {
    $st->bindValue(':nope', 1);
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
try {
    $db->prepare('SELECT ?, :a');
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
try {
    new PDO('pgsql:host=' . getenv('DBHOST') . ';port=' . getenv('DBPORT') . ';dbname=test', 'postgres', 'wrong');
} catch (PDOException $e) {
    // The server's address differs between where PHP and p8 ran.
    echo preg_replace('/server at "[^"]*"( \([^)]*\))?, port \d+/', 'server at HOST', $e->getMessage()), "\n";
}

$db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_WARNING);
var_dump($db->exec('DROP TABLE nope'));
var_dump($db->errorCode(), $db->errorInfo());
var_dump($db->lastInsertId('no_seq'));
$db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_SILENT);
var_dump($db->lastInsertId());
var_dump($db->errorInfo()[0]);
$db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION);

$db->exec('CREATE TEMP TABLE acct (id int PRIMARY KEY, v int)');
var_dump($db->inTransaction(), $db->beginTransaction(), $db->inTransaction());
$db->exec('INSERT INTO acct VALUES (1, 10)');
var_dump($db->rollBack(), $db->query('SELECT count(*) FROM acct')->fetchColumn());
$db->beginTransaction();
$db->exec('INSERT INTO acct VALUES (1, 10)');
try {
    $db->exec('INSERT INTO acct VALUES (1, 11)');
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
try {
    $db->query('SELECT 1');
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
var_dump($db->inTransaction(), $db->rollBack(), $db->inTransaction());

var_dump($db->escapeIdentifier('my "table"'));
var_dump($db->copyFromArray('acct', ["1\t10", "2\t\\N", "3\t30\n"]));
var_dump($db->copyToArray('acct'));
var_dump($db->copyToArray('acct', ',', 'NULL', 'v, id'));
var_dump($db->pgsqlCopyFromArray('acct', ["4;40"], ';'));
var_dump($db->query('SELECT sum(v) FROM acct')->fetchColumn());
try {
    $db->copyFromArray('acct', []);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
try {
    $db->copyFromArray('acct', ["1\t10"]);
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}

var_dump(is_int($db->getPid()), $db->getPid() === $db->pgsqlGetPid());
var_dump($db->getNotify());
$db->exec('LISTEN chan');
$db->exec("NOTIFY chan, 'hello'");
$n = $db->getNotify(PDO::FETCH_ASSOC);
var_dump($n['message'], $n['payload'], $n['pid'] === $db->getPid());
$db->exec('NOTIFY chan');
var_dump(count($db->getNotify(PDO::FETCH_NUM)), $db->getNotify(PDO::FETCH_BOTH, 10));

$db->setNoticeCallback(function ($m) { echo "notice: ", trim($m), "\n"; });
$db->exec("DO \$\$ BEGIN RAISE NOTICE 'hi %', 42; END \$\$");
$db->setNoticeCallback(null);
$db->exec("DO \$\$ BEGIN RAISE NOTICE 'quiet'; END \$\$");

$oid = $db->lobCreate();
var_dump(is_string($oid) && ctype_digit($oid), $db->lobUnlink($oid));
echo "done\n";
