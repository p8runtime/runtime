<?php
$db = new PDO('sqlite::memory:');
$db->exec('CREATE TABLE b (id INTEGER PRIMARY KEY, n INTEGER, f REAL, s TEXT, bl BLOB, z)');

// bindValue with explicit types
$st = $db->prepare('INSERT INTO b (n, f, s, bl, z) VALUES (:n, :f, :s, :bl, :z)');
$st->bindValue(':n', 42, PDO::PARAM_INT);
$st->bindValue(':f', 1.5);
$st->bindValue(':s', 'text', PDO::PARAM_STR);
$st->bindValue(':bl', "bin\0ary", PDO::PARAM_LOB);
$st->bindValue(':z', null, PDO::PARAM_NULL);
var_dump($st->execute());
// bool as int and null for any type
$st->bindValue(':n', true, PDO::PARAM_BOOL);
$st->bindValue(':f', null);
$st->bindValue(':s', 7);
$st->bindValue(':bl', 'plain');
$st->bindValue(':z', 3.25);
$st->execute();
foreach ($db->query('SELECT n, f, s, typeof(s) ts, bl, typeof(bl) tb, z, typeof(z) tz FROM b') as $row) {
    var_dump($row['n'], $row['f'], $row['s'], $row['ts'], bin2hex($row['bl']), $row['tb'], $row['z'], $row['tz']);
}

// bindParam binds the variable: it is read when execute() runs
$db->exec('DELETE FROM b');
$st = $db->prepare('INSERT INTO b (n, s) VALUES (:n, :s)');
$st->bindParam(':n', $n, PDO::PARAM_INT);
$st->bindParam(':s', $s);
foreach ([[1, 'one'], [2, 'two'], [3, 'three']] as [$n, $s]) {
    $st->execute();
}
var_dump($db->query('SELECT group_concat(n || s, ",") FROM b')->fetchColumn());

// positional bindParam, 1-based
$st = $db->prepare('SELECT s FROM b WHERE n = ? OR n = ? ORDER BY n');
$a = 1;
$b = 3;
$st->bindParam(1, $a, PDO::PARAM_INT);
$st->bindParam(2, $b, PDO::PARAM_INT);
$st->execute();
var_dump($st->fetchAll(PDO::FETCH_COLUMN));
$a = 2;
$st->execute();
var_dump($st->fetchAll(PDO::FETCH_COLUMN));

// execute() with an array replaces what was bound
$st->execute([1, 2]);
var_dump($st->fetchAll(PDO::FETCH_COLUMN));

// the variable bound by reference is converted by its type
$v = 5;
$st = $db->prepare('SELECT ?');
$st->bindParam(1, $v);
var_dump($v);

// bindColumn
$st = $db->prepare('SELECT n, s FROM b ORDER BY n');
$st->execute();
$st->bindColumn(1, $num, PDO::PARAM_INT);
$st->bindColumn(2, $str, PDO::PARAM_STR);
while ($st->fetch(PDO::FETCH_BOUND)) {
    echo $num, '=', $str, "\n";
}
$st->execute();
$st->bindColumn('s', $by_name);
$st->fetch(PDO::FETCH_BOUND);
var_dump($by_name);

// debugDumpParams
$st = $db->prepare('SELECT * FROM b WHERE n = :n AND s = ?1');
$st->bindValue(':n', 1, PDO::PARAM_INT);
$st->debugDumpParams();
$st = $db->prepare('SELECT * FROM b WHERE n = ?');
$st->bindValue(1, 7);
$st->debugDumpParams();

// blobs and nulls round-trip
$db->exec('DELETE FROM b');
$blob = "\x00\xff\xfe\x80abc";
$st = $db->prepare('INSERT INTO b (bl) VALUES (?)');
$st->bindValue(1, $blob, PDO::PARAM_LOB);
$st->execute();
$out = $db->query('SELECT bl FROM b')->fetchColumn();
var_dump($out === $blob, strlen($out));
$st = $db->prepare('SELECT bl FROM b');
$st->execute();
$st->bindColumn(1, $lob, PDO::PARAM_LOB);
$st->fetch(PDO::FETCH_BOUND);
var_dump(is_resource($lob), stream_get_contents($lob) === $blob);
