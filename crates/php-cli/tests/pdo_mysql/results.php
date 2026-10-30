<?php
// Result sets: fetch modes, rowCount(), multiple statements and
// nextRowset(), stored procedures, unbuffered queries, closeCursor().
$dsn = 'mysql:host=' . getenv('DBHOST') . ';port=' . getenv('DBPORT') . ';dbname=pdo_p8';
$db = new PDO($dsn, 'wp', 'wp');
$db->exec('DROP TABLE IF EXISTS results_t');
$db->exec('DROP PROCEDURE IF EXISTS results_p');
$db->exec('CREATE TABLE results_t (id INT AUTO_INCREMENT PRIMARY KEY, grp CHAR(1), v VARCHAR(10))');
var_dump($db->exec("INSERT INTO results_t (grp, v) VALUES ('a', 'one'), ('a', 'two'), ('b', 'three')"), $db->lastInsertId());
$db->exec("CREATE PROCEDURE results_p(IN x INT) BEGIN SELECT x AS first; SELECT v FROM results_t WHERE id <= x ORDER BY id; END");

function attempt(callable $f): void {
    try {
        var_dump($f());
    } catch (PDOException $e) {
        echo 'PDOException: ', $e->getMessage(), "\n";
        var_dump($e->getCode(), $e->errorInfo);
    }
}

class Row {
    public $id;
    public $v;
}

foreach ([true, false] as $emulate) {
    echo "--- emulate=", var_export($emulate, true), "\n";
    $db->setAttribute(PDO::ATTR_EMULATE_PREPARES, $emulate);
    $q = fn () => $db->query('SELECT id, grp, v FROM results_t ORDER BY id');
    var_dump($q()->fetch());
    print_r($q()->fetch(PDO::FETCH_OBJ));
    var_dump($q()->fetchAll(PDO::FETCH_CLASS, 'Row'));
    var_dump($q()->fetchAll(PDO::FETCH_GROUP | PDO::FETCH_COLUMN, 2));
    var_dump($db->query('SELECT id, v FROM results_t')->fetchAll(PDO::FETCH_KEY_PAIR));
    var_dump($db->query('SELECT v, id, grp FROM results_t')->fetchAll(PDO::FETCH_UNIQUE));
    var_dump($db->query('SELECT 1 AS a, 2 AS a')->fetch(PDO::FETCH_NAMED));
    foreach ($db->query('SELECT id FROM results_t ORDER BY id', PDO::FETCH_NUM) as $k => $row) {
        echo $k, ': ', $row[0], "\n";
    }
    $st = $db->prepare('SELECT id, v FROM results_t WHERE grp = ?');
    $st->execute(['a']);
    var_dump($st->rowCount(), $st->columnCount());
    $st->bindColumn('v', $v);
    while ($st->fetch(PDO::FETCH_BOUND)) {
        echo $v, "\n";
    }
    var_dump($st->fetch(), $st->errorCode());
    $st = $db->prepare('UPDATE results_t SET v = v WHERE grp = :g');
    $st->execute([':g' => 'a']);
    var_dump($st->rowCount(), $st->columnCount());
    $st = $db->prepare('DELETE FROM results_t WHERE id = 0');
    $st->execute();
    var_dump($st->rowCount());

    // Several statements in one string: one result each (a server-side
    // prepare takes one statement only).
    attempt(function () use ($db) {
        $st = $db->prepare('SELECT ?; SELECT 6');
        return $st->execute([5]) ? $st->fetchAll(PDO::FETCH_NUM) : false;
    });
    attempt(fn () => $db->query('SELECT 7')->fetchColumn());
    if ($emulate) {
        $st = $db->query("SELECT 1 AS a; SELECT 'x' AS b, 'y' AS c; UPDATE results_t SET v = v; SELECT id FROM results_t WHERE id = 3");
        do {
            var_dump($st->columnCount(), $st->rowCount(), $st->fetchAll(PDO::FETCH_ASSOC));
        } while ($st->nextRowset());
        var_dump($st->nextRowset());
        // The other results must be read (or the cursor closed) first.
        $st = $db->query('SELECT 1; SELECT 2');
        attempt(fn () => $db->query('SELECT 3'));
        var_dump($st->closeCursor());
        var_dump($db->query('SELECT 4')->fetchColumn());
        // An error in a later statement shows at nextRowset().
        $st = $db->query('SELECT 1; SELECT * FROM no_such_table; SELECT 2');
        var_dump($st->fetchAll(PDO::FETCH_NUM));
        attempt(fn () => $st->nextRowset());
        attempt(fn () => $db->query('SELECT 8')->fetchColumn());
    }

    // A stored procedure: its result sets, then the status of the CALL.
    $st = $db->prepare('CALL results_p(?)');
    $st->execute([2]);
    do {
        var_dump($st->columnCount(), $st->fetchAll(PDO::FETCH_NUM));
    } while ($st->nextRowset());
    attempt(fn () => $db->query('SELECT 9')->fetchColumn());

    // Unbuffered: rows come as they are fetched; nothing else may run meanwhile.
    $db->setAttribute(PDO::MYSQL_ATTR_USE_BUFFERED_QUERY, false);
    $st = $db->query('SELECT id FROM results_t ORDER BY id');
    var_dump($st->rowCount(), $st->fetchColumn());
    attempt(fn () => $db->query('SELECT 10'));
    attempt(fn () => $db->exec('SET @a = 1'));
    var_dump($st->fetchAll(PDO::FETCH_COLUMN), $st->rowCount());
    var_dump($db->query('SELECT 11')->fetchColumn());
    $st = $db->query('SELECT id FROM results_t ORDER BY id');
    $st->fetch();
    var_dump($st->closeCursor());
    var_dump($db->query('SELECT 12')->fetchColumn());
    $st = $db->query('SELECT id FROM results_t ORDER BY id');
    $st->fetch();
    unset($st);
    attempt(fn () => $db->query('SELECT 13')->fetchColumn());
    $st = $db->prepare('SELECT id FROM results_t WHERE id > ? ORDER BY id');
    $st->execute([1]);
    var_dump($st->rowCount(), $st->fetchAll(PDO::FETCH_COLUMN), $st->rowCount());
    $db->setAttribute(PDO::MYSQL_ATTR_USE_BUFFERED_QUERY, true);
    $st = $db->prepare('SELECT id FROM results_t ORDER BY id', [PDO::MYSQL_ATTR_USE_BUFFERED_QUERY => false]);
    $st->execute();
    var_dump($st->rowCount());
    $st->closeCursor();

    // lastInsertId() follows the last statement that made one.
    $db->exec("INSERT INTO results_t (grp, v) VALUES ('c', 'four')");
    $id = $db->lastInsertId();
    $db->query('SELECT 1')->fetchAll();
    var_dump($id === $db->lastInsertId(), $db->lastInsertId('ignored') === $id);
    $db->exec('DELETE FROM results_t WHERE grp = "c"');
}

// An error in a result set closeCursor() drops is reported.
$db->setAttribute(PDO::ATTR_EMULATE_PREPARES, true);
$st = $db->query('SELECT 1; SELECT x FROM no_such_table');
var_dump($st->fetchAll(PDO::FETCH_NUM));
attempt(fn () => $st->closeCursor());
// A stream bound as a LOB to a server-side prepare: the variable becomes
// what was read from it.
$db->setAttribute(PDO::ATTR_EMULATE_PREPARES, false);
$fp = fopen('php://memory', 'w+');
fwrite($fp, 'streamed');
rewind($fp);
$st = $db->prepare('SELECT ?');
$st->bindParam(1, $fp, PDO::PARAM_LOB);
$st->execute();
var_dump($st->fetchColumn(), $fp);
$db->exec('DROP PROCEDURE results_p');
$db->exec('DROP TABLE results_t');
