<?php
// Column types as PHP gets them: text protocol (emulated prepares) and
// binary protocol (server-side prepares), with and without
// PDO::ATTR_STRINGIFY_FETCHES, and getColumnMeta().
$db = new PDO('mysql:host=' . getenv('DBHOST') . ';port=' . getenv('DBPORT') . ';dbname=pdo_p8', 'wp', 'wp');
$db->exec('DROP TABLE IF EXISTS types_t');
$db->exec('CREATE TABLE types_t (
    id INT AUTO_INCREMENT PRIMARY KEY, ti TINYINT, uti TINYINT UNSIGNED, si SMALLINT UNSIGNED, mi MEDIUMINT,
    i INT NOT NULL DEFAULT 0, ui INT UNSIGNED, bi BIGINT, ubi BIGINT UNSIGNED,
    f FLOAT, f2 FLOAT(7,4), d DOUBLE, d2 DOUBLE, dec1 DECIMAL(10,3), dec0 DECIMAL(20,0),
    s VARCHAR(20) UNIQUE, c CHAR(3), tx TEXT, mt MEDIUMTEXT, b BLOB, lb LONGBLOB, bn BINARY(4), vb VARBINARY(10),
    dt DATETIME, dt3 DATETIME(3), dt6 DATETIME(6), da DATE, tm TIME, tm2 TIME(2), ts TIMESTAMP NULL, y YEAR,
    bt BIT(10), bt64 BIT(64), e ENUM("a","b"), st SET("x","y"), j JSON, z INT(5) ZEROFILL, n INT NULL, bo BOOLEAN,
    KEY (mi)
)');
$db->exec("INSERT INTO types_t VALUES (NULL, -5, 255, 65535, -8388608, 2147483647, 4294967295, -9223372036854775808, 18446744073709551615,
    1.1, 123.4567, 2.5, 0.000025, 3.14159, 12345678901234567890,
    'héllo', 'ab', 'text', 'medium', 0x00FF61, 0x0102, 'ab', 'vb',
    '2024-01-02 03:04:05', '2024-01-02 03:04:05.678', '2024-01-02 03:04:05.123456', '2024-01-02', '-838:59:59', '12:34:56.78', NULL, 2024,
    b'1010101010', b'1111111111111111111111111111111111111111111111111111111111111111', 'b', 'x,y', '{\"a\": 1}', 42, NULL, TRUE)");
$db->exec("INSERT INTO types_t (id, i, f, d, dt, da, tm, ts, s) VALUES (2, -1, -0.5, 1e300, '0000-00-00 00:00:00', '0000-00-00', '00:00:00', '2000-01-01 00:00:00', 'two')");
var_dump($db->lastInsertId());

foreach ([true, false] as $emulate) {
    foreach ([false, true] as $stringify) {
        $db->setAttribute(PDO::ATTR_EMULATE_PREPARES, $emulate);
        $db->setAttribute(PDO::ATTR_STRINGIFY_FETCHES, $stringify);
        echo "emulate=", var_export($emulate, true), " stringify=", var_export($stringify, true), "\n";
        $st = $db->prepare('SELECT * FROM types_t ORDER BY id');
        $st->execute();
        var_dump($st->rowCount(), $st->columnCount());
        foreach ($st->fetchAll(PDO::FETCH_ASSOC) as $row) {
            foreach ($row as $k => $v) {
                echo $k, '=', is_string($v) ? 'string:' . bin2hex($v) : var_export($v, true), ' ';
            }
            echo "\n";
        }
    }
}
$db->setAttribute(PDO::ATTR_STRINGIFY_FETCHES, false);

foreach ([true, false] as $emulate) {
    $db->setAttribute(PDO::ATTR_EMULATE_PREPARES, $emulate);
    $st = $db->query('SELECT * FROM types_t');
    for ($i = 0; $i < $st->columnCount(); $i++) {
        echo json_encode($st->getColumnMeta($i)), "\n";
    }
    var_dump($st->getColumnMeta(99));
    $st = $db->query('SELECT 1, 1.5, 1e0, NULL, "x", COUNT(*), 1 + 1 AS sum, CAST(1 AS UNSIGNED), -1, 9223372036854775807, 18446744073709551615, 0.1 + 0.2, PI(), CAST(1.5 AS FLOAT), CAST(1.5 AS DOUBLE), NOW() > 0, X\'41\', DATE("2020-01-01"), CAST("12:00:00" AS TIME), CONCAT(1) FROM types_t WHERE id = 1');
    var_dump($st->fetch(PDO::FETCH_NUM));
    for ($i = 0; $i < $st->columnCount(); $i++) {
        echo json_encode($st->getColumnMeta($i)), "\n";
    }
}

// Values bound and read back, both ways.
foreach ([true, false] as $emulate) {
    $db->setAttribute(PDO::ATTR_EMULATE_PREPARES, $emulate);
    $st = $db->prepare('SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?');
    $st->bindValue(1, 42, PDO::PARAM_INT);
    $st->bindValue(2, '42', PDO::PARAM_INT);
    $st->bindValue(3, 1.5);
    $st->bindValue(4, true, PDO::PARAM_BOOL);
    $st->bindValue(5, false, PDO::PARAM_BOOL);
    $st->bindValue(6, null, PDO::PARAM_INT);
    $st->bindValue(7, "a'b\\c\0d");
    $st->bindValue(8, 'x', PDO::PARAM_NULL);
    $st->bindValue(9, -9223372036854775807 - 1, PDO::PARAM_INT);
    $st->bindValue(10, 7, PDO::PARAM_STR);
    $st->execute();
    var_dump($st->fetch(PDO::FETCH_NUM));
    $st = $db->prepare('SELECT ? + 0, ? + 0.0, CONCAT(?), ?');
    $st->execute([1.25, '3', 5, 'é']);
    var_dump($st->fetch(PDO::FETCH_NUM));
}

// LOB columns and streams.
foreach ([true, false] as $emulate) {
    $db->setAttribute(PDO::ATTR_EMULATE_PREPARES, $emulate);
    $fp = fopen('php://memory', 'w+');
    fwrite($fp, "\x00blob\xffdata");
    rewind($fp);
    $st = $db->prepare('UPDATE types_t SET lb = ? WHERE id = 2');
    $st->bindParam(1, $fp, PDO::PARAM_LOB);
    $st->execute();
    var_dump($st->rowCount());
    $st = $db->query('SELECT lb, b FROM types_t WHERE id = 2');
    $st->bindColumn(1, $lob, PDO::PARAM_LOB);
    $st->bindColumn(2, $plain, PDO::PARAM_STR);
    $st->fetch(PDO::FETCH_BOUND);
    var_dump(is_resource($lob) ? bin2hex(stream_get_contents($lob)) : $lob, $plain);
    var_dump(bin2hex($db->quote("\x00\xff", PDO::PARAM_LOB)));
}

// Table names in column names.
$db->setAttribute(PDO::ATTR_FETCH_TABLE_NAMES, true);
var_dump($db->query('SELECT id, 1 AS one FROM types_t WHERE id = 1')->fetch(PDO::FETCH_ASSOC));
$db->setAttribute(PDO::ATTR_FETCH_TABLE_NAMES, false);
$db->setAttribute(PDO::ATTR_CASE, PDO::CASE_UPPER);
var_dump($db->query('SELECT id, s FROM types_t WHERE id = 1')->fetch(PDO::FETCH_ASSOC));
$db->exec('DROP TABLE types_t');
