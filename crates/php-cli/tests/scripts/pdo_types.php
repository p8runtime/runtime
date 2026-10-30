<?php
$db = new PDO('sqlite::memory:');
$db->exec('CREATE TABLE ty (id INTEGER PRIMARY KEY, a, b)');
$vals = [0, -1, PHP_INT_MAX, PHP_INT_MIN, 1.5, 0.1, 1e100, -2.5e-10, '', '0', 'héllo wörld', "line\nbreak", null];
$st = $db->prepare('INSERT INTO ty (a, b) VALUES (?, ?)');
foreach ($vals as $v) {
    $st->bindValue(1, $v, is_int($v) ? PDO::PARAM_INT : (is_null($v) ? PDO::PARAM_NULL : PDO::PARAM_STR));
    $st->bindValue(2, $v, is_int($v) ? PDO::PARAM_INT : (is_null($v) ? PDO::PARAM_NULL : PDO::PARAM_STR));
    $st->execute();
}
// literals keep their SQLite types
$db->exec("INSERT INTO ty (a, b) VALUES (12, 3.0), (X'00ff10', 'text'), (1e3, -0.5), (9223372036854775807, 9223372036854775808)");

echo "== native\n";
foreach ($db->query('SELECT a, typeof(a) ta, b FROM ty ORDER BY id') as $r) {
    echo json_encode([$r['a'], $r['ta'], $r['b']], JSON_PARTIAL_OUTPUT_ON_ERROR), ' ', gettype($r['a']), ' ', gettype($r['b']), "\n";
}

echo "== stringify\n";
$db->setAttribute(PDO::ATTR_STRINGIFY_FETCHES, true);
foreach ($db->query('SELECT a, b FROM ty ORDER BY id LIMIT 6') as $r) {
    echo gettype($r['a']), ':', var_export($r['a'], true), ' ', gettype($r['b']), ':', var_export($r['b'], true), "\n";
}
var_dump($db->getAttribute(PDO::ATTR_STRINGIFY_FETCHES));
$db->setAttribute(PDO::ATTR_STRINGIFY_FETCHES, false);

echo "== case\n";
$db->exec('CREATE TABLE cs (MixedCase INTEGER, lower INTEGER, UPPER INTEGER)');
$db->exec('INSERT INTO cs VALUES (1, 2, 3)');
foreach ([PDO::CASE_NATURAL, PDO::CASE_LOWER, PDO::CASE_UPPER] as $c) {
    $db->setAttribute(PDO::ATTR_CASE, $c);
    echo json_encode($db->query('SELECT * FROM cs')->fetch(PDO::FETCH_ASSOC)), "\n";
}
var_dump($db->getAttribute(PDO::ATTR_CASE));
$db->setAttribute(PDO::ATTR_CASE, PDO::CASE_NATURAL);

echo "== oracle nulls\n";
$db->exec("CREATE TABLE on_ (a, b)");
$db->exec("INSERT INTO on_ VALUES ('', NULL)");
foreach ([PDO::NULL_NATURAL, PDO::NULL_EMPTY_STRING, PDO::NULL_TO_STRING] as $m) {
    $db->setAttribute(PDO::ATTR_ORACLE_NULLS, $m);
    echo json_encode($db->query('SELECT * FROM on_')->fetch(PDO::FETCH_NUM)), "\n";
}
$db->setAttribute(PDO::ATTR_ORACLE_NULLS, PDO::NULL_NATURAL);

echo "== column meta for each type\n";
$st = $db->query("SELECT 1 AS i, 1.5 AS f, 'x' AS s, X'00' AS b, NULL AS n");
for ($i = 0; $i < 5; $i++) {
    $m = $st->getColumnMeta($i);
    echo $m['name'], ' ', $m['native_type'], ' ', $m['pdo_type'], ' ', json_encode($m['flags']), "\n";
}

echo "== misc attributes\n";
var_dump($db->getAttribute(PDO::ATTR_PERSISTENT));
var_dump($db->getAttribute(PDO::ATTR_STATEMENT_CLASS));
var_dump($db->setAttribute(PDO::ATTR_TIMEOUT, 5));
var_dump($db->prepare('SELECT 1')->getAttribute(PDO::SQLITE_ATTR_READONLY_STATEMENT));
var_dump($db->prepare('DELETE FROM ty')->getAttribute(Pdo\Sqlite::ATTR_READONLY_STATEMENT));
var_dump(PDO::SQLITE_ATTR_OPEN_FLAGS, Pdo\Sqlite::OPEN_READONLY, Pdo\Sqlite::DETERMINISTIC);
var_dump(PDO::FETCH_ASSOC, PDO::FETCH_GROUP, PDO::FETCH_UNIQUE, PDO::PARAM_INT, PDO::ERR_NONE);
