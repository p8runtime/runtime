<?php
// Types, placeholders and prepares against PostgreSQL.
$db = new PDO('pgsql:host=' . getenv('DBHOST') . ';port=' . getenv('DBPORT') . ';dbname=test', 'postgres', 'p8');
var_dump($db->getAttribute(PDO::ATTR_DRIVER_NAME), $db->getAttribute(PDO::ATTR_ERRMODE));
var_dump($db->getAttribute(PDO::ATTR_EMULATE_PREPARES), $db->getAttribute(PDO::PGSQL_ATTR_DISABLE_PREPARES));
var_dump($db->getAttribute(PDO::ATTR_CONNECTION_STATUS));

$row = $db->query("SELECT 1 AS i, 9223372036854775807::int8 AS big, 2::int2 AS small, true AS t, false AS f,
    1.5::float8 AS d, 'Infinity'::float4 AS inf, 12.50::numeric AS n, 'text' AS s, NULL AS z,
    '\\x00ff41'::bytea AS b, 42::oid AS o, '2024-01-02'::date AS dt, ARRAY[1,2] AS arr, '{\"a\":1}'::jsonb AS j")->fetch(PDO::FETCH_ASSOC);
foreach ($row as $k => $v) {
    echo $k, ': ';
    if (is_resource($v)) {
        echo 'stream ', bin2hex(stream_get_contents($v)), "\n";
    } else {
        var_dump($v);
    }
}

$db->exec('CREATE TEMP TABLE t (id serial PRIMARY KEY, name text, flag boolean, data bytea, amount numeric(10,2))');
$ins = $db->prepare('INSERT INTO t (name, flag, data, amount) VALUES (?, ?, ?, ?)');
$ins->bindValue(1, 'alice');
$ins->bindValue(2, true, PDO::PARAM_BOOL);
$ins->bindValue(3, "\x00\x01bin", PDO::PARAM_LOB);
$ins->bindValue(4, '10.25');
var_dump($ins->execute(), $ins->rowCount());
var_dump($db->lastInsertId(), $db->lastInsertId('t_id_seq'));

$ins = $db->prepare('INSERT INTO t (name, flag, amount) VALUES (:name, :flag, :amount)');
var_dump($ins->execute([':name' => 'bob', ':flag' => 'f', ':amount' => 3]));
try {
    // false binds as a string, "", which is no boolean.
    $ins->execute(['name' => 'carol', 'flag' => false, 'amount' => null]);
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
var_dump($ins->execute(['name' => 'carol', 'flag' => '0', 'amount' => null]));

$sel = $db->prepare('SELECT id, name, flag, amount FROM t WHERE name = :n OR name = :n2 OR id = :n3 ORDER BY id');
$sel->execute(['n' => 'alice', 'n2' => 'carol', 'n3' => 2]);
var_dump($sel->fetchAll(PDO::FETCH_ASSOC));

// The same name twice reuses its $N.
$st = $db->prepare('SELECT :a::int + :a::int AS two_a, :b::text AS b');
$st->execute(['a' => 21, 'b' => 'x']);
var_dump($st->fetch(PDO::FETCH_ASSOC));

// Blobs come back as streams.
$st = $db->query('SELECT data FROM t WHERE data IS NOT NULL');
$st->bindColumn(1, $lob, PDO::PARAM_LOB);
$st->fetch(PDO::FETCH_BOUND);
var_dump(is_resource($lob), bin2hex(stream_get_contents($lob)));
$st = $db->query('SELECT data FROM t WHERE data IS NOT NULL');
$st->bindColumn(1, $str, PDO::PARAM_STR);
$st->fetch(PDO::FETCH_BOUND);
var_dump(bin2hex($str));

// Emulated prepares and PGSQL_ATTR_DISABLE_PREPARES.
foreach ([[PDO::ATTR_EMULATE_PREPARES => true], [PDO::PGSQL_ATTR_DISABLE_PREPARES => true], []] as $opts) {
    $st = $db->prepare("SELECT ?::text AS q, ? AS b, '??' AS lit", $opts);
    $st->bindValue(1, "it's");
    $st->bindValue(2, true, PDO::PARAM_BOOL);
    $st->execute();
    var_dump($st->getAttribute(PDO::ATTR_EMULATE_PREPARES), $st->fetch(PDO::FETCH_NUM));
}
$db->setAttribute(PDO::ATTR_EMULATE_PREPARES, true);
$st = $db->prepare('SELECT count(*) FROM t WHERE name IN (?, ?)');
$st->execute(['alice', 'bob']);
var_dump($st->fetchColumn());
$db->setAttribute(PDO::ATTR_EMULATE_PREPARES, false);

var_dump($db->quote("it's"), $db->quote("a\\b"), $db->quote("\x00\xff", PDO::PARAM_LOB));
var_dump($db->exec("UPDATE t SET flag = NOT flag"), $db->exec('SELECT 1'));

$st = $db->query('SELECT id, name, amount, flag, data FROM t LIMIT 1');
for ($i = 0; $i < $st->columnCount(); $i++) {
    $m = $st->getColumnMeta($i);
    unset($m['pgsql:table_oid']);
    echo json_encode($m), "\n";
}

$db->setAttribute(PDO::ATTR_STRINGIFY_FETCHES, true);
var_dump($db->query('SELECT 1 AS i, true AS b, 1.5::float8 AS f')->fetch(PDO::FETCH_ASSOC));
