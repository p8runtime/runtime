<?php
// Placeholders and bound values, with emulated prepares (the default) and
// server-side prepares: named, positional, bindParam by reference,
// execute() arrays, missing and extra parameters.
$db = new PDO('mysql:host=' . getenv('DBHOST') . ';port=' . getenv('DBPORT') . ';dbname=pdo_p8', 'wp', 'wp');
$db->exec('DROP TABLE IF EXISTS binds_t');
$db->exec('CREATE TABLE binds_t (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(20), n INT, f DOUBLE)');

function attempt(callable $f): void {
    try {
        $r = $f();
        var_dump($r);
    } catch (PDOException $e) {
        echo 'PDOException: ', $e->getMessage(), "\n";
        var_dump($e->getCode(), $e->errorInfo);
    }
}

foreach ([true, false] as $emulate) {
    echo "--- emulate=", var_export($emulate, true), "\n";
    $db->setAttribute(PDO::ATTR_EMULATE_PREPARES, $emulate);
    $db->exec('DELETE FROM binds_t');
    $db->exec('ALTER TABLE binds_t AUTO_INCREMENT = 1');

    $ins = $db->prepare('INSERT INTO binds_t (name, n, f) VALUES (:name, :n, :f)');
    foreach ([['a', 1, 1.5], ['b', 2, -2.25], ["o'k", null, 0.0]] as [$name, $n, $f]) {
        $ins->bindValue(':name', $name);
        $ins->bindValue('n', $n, $n === null ? PDO::PARAM_NULL : PDO::PARAM_INT);
        $ins->bindValue(':f', $f);
        var_dump($ins->execute(), $ins->rowCount(), $db->lastInsertId());
    }
    $ins = $db->prepare('INSERT INTO binds_t (name, n) VALUES (?, ?)');
    $name = 'ref';
    $n = 10;
    $ins->bindParam(1, $name);
    $ins->bindParam(2, $n, PDO::PARAM_INT);
    $ins->execute();
    $name = 'ref2';
    $n = 11;
    $ins->execute();
    var_dump($ins->execute(['arr', 12]), $db->lastInsertId());
    var_dump($db->query('SELECT id, name, n, f FROM binds_t ORDER BY id')->fetchAll(PDO::FETCH_NUM));

    $sel = $db->prepare('SELECT name FROM binds_t WHERE n > ? ORDER BY id LIMIT ?');
    $sel->bindValue(1, 1, PDO::PARAM_INT);
    $sel->bindValue(2, 2, PDO::PARAM_INT);
    $sel->execute();
    var_dump($sel->fetchAll(PDO::FETCH_COLUMN));
    // LIMIT '1' is a syntax error; the binary protocol sends a string too.
    attempt(fn () => $sel->execute([0, 1]) ? $sel->fetchAll(PDO::FETCH_COLUMN) : false);

    // A name used twice.
    $twice = $db->prepare('SELECT :v AS a, :v AS b');
    attempt(fn () => $twice->execute([':v' => 'x']));
    attempt(fn () => $twice->fetch(PDO::FETCH_ASSOC));

    // Too few, too many, unknown and mixed.
    attempt(fn () => $db->prepare('SELECT ?, ?')->execute([1]));
    attempt(fn () => $db->prepare('SELECT ?')->execute([1, 2]));
    attempt(fn () => $db->prepare('SELECT :a')->execute([':b' => 1]));
    attempt(fn () => $db->prepare('SELECT ?, :a'));
    attempt(fn () => $db->prepare('SELECT ?')->bindValue(2, 1));
    attempt(fn () => $db->prepare('SELECT :a')->bindValue(':nope', 1));
    attempt(fn () => $db->prepare('SELECT :a')->bindValue(1, 'by number'));
    $st = $db->prepare('SELECT :a');
    $st->bindValue(1, 'by number');
    attempt(fn () => $st->execute() ? $st->fetchColumn() : false);
    $st = $db->prepare('SELECT ?');
    attempt(fn () => $st->execute());

    // Placeholders inside strings, comments and identifiers are text.
    $st = $db->prepare("SELECT '?', \"?\", 1 AS `?x`, ? /* ? :a */ -- ?\n, ? # :b\n");
    $st->execute([1, 2]);
    var_dump($st->fetch(PDO::FETCH_NUM));
    $st = $db->prepare("SELECT 'it\\'s ?', ?");
    $st->execute(['ok']);
    var_dump($st->fetch(PDO::FETCH_NUM));

    // No placeholders at all, and values given anyway.
    attempt(fn () => $db->prepare('SELECT 1')->execute([1]));

    // Statements the binary protocol cannot prepare (1295) are emulated.
    $db->exec('SET @v = 5');
    $st = $db->prepare("PREPARE p8s FROM 'SELECT ? + 1 AS r'");
    var_dump($st->execute());
    $st = $db->prepare('EXECUTE p8s USING @v');
    $st->execute();
    var_dump($st->fetchAll(PDO::FETCH_ASSOC));
    $db->exec('DEALLOCATE PREPARE p8s');

    // Reusing a statement after a partial fetch.
    $st = $db->prepare('SELECT id FROM binds_t WHERE id <= ? ORDER BY id');
    $st->execute([3]);
    var_dump($st->fetchColumn());
    $st->execute([2]);
    var_dump($st->fetchAll(PDO::FETCH_COLUMN));

    var_dump($db->quote("a'b\"c\\d\0e\n\r\x1a"), $db->quote('x', PDO::PARAM_INT), $db->quote('x', PDO::PARAM_STR | PDO::PARAM_STR_NATL));
    $st = $db->prepare('SELECT ?');
    $st->bindValue(1, 'nat', PDO::PARAM_STR | PDO::PARAM_STR_NATL);
    $st->execute();
    var_dump($st->fetchColumn());
    $st->debugDumpParams();
}
$db->exec('DROP TABLE binds_t');
