<?php
// Connecting: DSN keys, failures, Pdo\Mysql, the connection attributes and
// the PDO::MYSQL_ATTR_* options against MariaDB.
$h = getenv('DBHOST');
$p = getenv('DBPORT');
$dsn = "mysql:host=$h;port=$p;dbname=pdo_p8";
// The client's address is in some server messages; it differs per host.
function mask(string $s): string {
    return preg_replace("/'@'[^']*'/", "'@'%'", $s);
}
function show(Throwable $e): void {
    echo get_class($e), ': ', mask($e->getMessage()), "\n";
    if ($e instanceof PDOException) {
        var_dump($e->getCode());
        echo mask(json_encode($e->errorInfo)), "\n";
    }
}

print_r(PDO::getAvailableDrivers());
foreach ([
    [$dsn, 'wp', 'nope'],
    ["mysql:host=$h;port=$p;dbname=pdo_no_such_db", 'wp', 'wp'],
    ["mysql:host=127.0.0.1;port=1", 'wp', 'wp'],
    ["mysql:host=localhost;dbname=x", 'wp', 'wp'],
    ["mysql:unix_socket=/nonexistent/mysql.sock", 'wp', 'wp'],
    ["mysql:host=$h;port=$p;charset=nonesuch", 'wp', 'wp'],
    ["mysql:host=$h;port=$p;dbname=pdo_p8;user=wp;password=wp", null, null],
] as [$d, $u, $pw]) {
    try {
        $db = new PDO($d, $u, $pw);
        echo "connected\n";
    } catch (PDOException $e) {
        show($e);
    }
}

$db = new PDO($dsn, 'wp', 'wp');
echo get_class($db), "\n";
$my = Pdo\Mysql::connect($dsn, 'wp', 'wp');
echo get_class($my), ' ', get_class(PDO::connect($dsn, 'wp', 'wp')), "\n";
var_dump($my instanceof PDO, $my->getWarningCount());
$my->query('SELECT CAST("x" AS INT)')->fetchAll();
var_dump($my->getWarningCount());
try {
    Pdo\Sqlite::connect($dsn, 'wp', 'wp');
} catch (PDOException $e) {
    show($e);
}
var_dump(Pdo\Mysql::ATTR_USE_BUFFERED_QUERY, Pdo\Mysql::ATTR_INIT_COMMAND, Pdo\Mysql::ATTR_MULTI_STATEMENTS, PDO::MYSQL_ATTR_FOUND_ROWS);

var_dump($db->getAttribute(PDO::ATTR_DRIVER_NAME), $db->getAttribute(PDO::ATTR_CLIENT_VERSION));
echo str_replace($h, 'HOST', $db->getAttribute(PDO::ATTR_CONNECTION_STATUS)), "\n";
echo preg_replace('/\d+/', 'N', $db->getAttribute(PDO::ATTR_SERVER_INFO)), "\n";
var_dump($db->getAttribute(PDO::ATTR_SERVER_VERSION) === $db->query('SELECT VERSION()')->fetchColumn());
foreach ([
    'AUTOCOMMIT' => PDO::ATTR_AUTOCOMMIT,
    'DEFAULT_STR_PARAM' => PDO::ATTR_DEFAULT_STR_PARAM,
    'USE_BUFFERED_QUERY' => PDO::MYSQL_ATTR_USE_BUFFERED_QUERY,
    'LOCAL_INFILE' => PDO::MYSQL_ATTR_LOCAL_INFILE,
    'LOCAL_INFILE_DIRECTORY' => PDO::MYSQL_ATTR_LOCAL_INFILE_DIRECTORY,
    'FETCH_TABLE_NAMES' => PDO::ATTR_FETCH_TABLE_NAMES,
    'DIRECT_QUERY' => PDO::MYSQL_ATTR_DIRECT_QUERY,
    'EMULATE_PREPARES' => PDO::ATTR_EMULATE_PREPARES,
    'STRINGIFY_FETCHES' => PDO::ATTR_STRINGIFY_FETCHES,
    'ERRMODE' => PDO::ATTR_ERRMODE,
    'INIT_COMMAND' => PDO::MYSQL_ATTR_INIT_COMMAND,
    'FOUND_ROWS' => PDO::MYSQL_ATTR_FOUND_ROWS,
    'MULTI_STATEMENTS' => PDO::MYSQL_ATTR_MULTI_STATEMENTS,
    'SSL_CA' => PDO::MYSQL_ATTR_SSL_CA,
    'TIMEOUT' => PDO::ATTR_TIMEOUT,
] as $name => $a) {
    try {
        echo "$name: ";
        var_dump($db->getAttribute($a));
    } catch (PDOException $e) {
        show($e);
    }
}
foreach ([
    'INIT_COMMAND' => [PDO::MYSQL_ATTR_INIT_COMMAND, 'SET @x = 1'],
    'FOUND_ROWS' => [PDO::MYSQL_ATTR_FOUND_ROWS, true],
    'TIMEOUT' => [PDO::ATTR_TIMEOUT, 5],
    'LOCAL_INFILE' => [PDO::MYSQL_ATTR_LOCAL_INFILE, true],
    'USE_BUFFERED_QUERY' => [PDO::MYSQL_ATTR_USE_BUFFERED_QUERY, false],
    'DIRECT_QUERY' => [PDO::MYSQL_ATTR_DIRECT_QUERY, false],
    'DEFAULT_STR_PARAM' => [PDO::ATTR_DEFAULT_STR_PARAM, PDO::PARAM_STR_NATL],
    'AUTOCOMMIT' => [PDO::ATTR_AUTOCOMMIT, false],
] as $name => [$a, $v]) {
    echo "set $name: ";
    var_dump($db->setAttribute($a, $v));
    try {
        var_dump($db->getAttribute($a));
    } catch (PDOException $e) {
        echo $e->getMessage(), "\n";
    }
}
var_dump($db->getAttribute(PDO::ATTR_EMULATE_PREPARES), $db->quote("it's"));
try {
    $db->setAttribute(PDO::MYSQL_ATTR_USE_BUFFERED_QUERY, []);
} catch (TypeError $e) {
    show($e);
}

// Constructor options.
$o = new PDO($dsn, 'wp', 'wp', [
    PDO::MYSQL_ATTR_INIT_COMMAND => "SET @p8 = 'init', time_zone = '+05:00'",
    PDO::ATTR_EMULATE_PREPARES => false,
    PDO::MYSQL_ATTR_USE_BUFFERED_QUERY => false,
    PDO::ATTR_AUTOCOMMIT => false,
    PDO::ATTR_ERRMODE => PDO::ERRMODE_SILENT,
]);
var_dump($o->query('SELECT @p8, @@time_zone, @@autocommit')->fetchAll(PDO::FETCH_NUM));
var_dump($o->getAttribute(PDO::ATTR_EMULATE_PREPARES), $o->getAttribute(PDO::MYSQL_ATTR_USE_BUFFERED_QUERY), $o->getAttribute(PDO::ATTR_AUTOCOMMIT), $o->getAttribute(PDO::ATTR_ERRMODE));
try {
    new PDO($dsn, 'wp', 'wp', [PDO::MYSQL_ATTR_INIT_COMMAND => 'SELEC 1']);
} catch (PDOException $e) {
    show($e);
}

// The character set from the DSN.
foreach (['utf8mb4', 'latin1', 'utf8'] as $cs) {
    $c = new PDO("$dsn;charset=$cs", 'wp', 'wp');
    var_dump($c->query('SELECT @@character_set_client, @@character_set_results, @@character_set_connection')->fetch(PDO::FETCH_NUM));
}
$c = new PDO("$dsn;charset=latin1", 'wp', 'wp');
var_dump(bin2hex($c->query("SELECT 'é'")->fetchColumn()), $c->quote("\xe9'"));

// FOUND_ROWS: an UPDATE that changes nothing still counts its matched rows.
$db = new PDO($dsn, 'wp', 'wp');
$db->exec('DROP TABLE IF EXISTS conn_t');
$db->exec('CREATE TABLE conn_t (id INT PRIMARY KEY, v INT)');
$db->exec('INSERT INTO conn_t VALUES (1, 1), (2, 1)');
var_dump($db->exec('UPDATE conn_t SET v = 1'));
$f = new PDO($dsn, 'wp', 'wp', [PDO::MYSQL_ATTR_FOUND_ROWS => true]);
var_dump($f->exec('UPDATE conn_t SET v = 1'));
$st = $f->prepare('UPDATE conn_t SET v = ?');
$st->execute([1]);
var_dump($st->rowCount());

// MULTI_STATEMENTS off: two statements in one string are a syntax error.
$m = new PDO($dsn, 'wp', 'wp', [PDO::MYSQL_ATTR_MULTI_STATEMENTS => false]);
try {
    $m->exec('SELECT 1; SELECT 2');
} catch (PDOException $e) {
    show($e);
}
var_dump($db->exec('UPDATE conn_t SET v = 2 WHERE id = 1; UPDATE conn_t SET v = 3'));
var_dump($db->query('SELECT v FROM conn_t ORDER BY id')->fetchAll(PDO::FETCH_COLUMN));

// LOAD DATA LOCAL INFILE is refused.
try {
    $db->exec("LOAD DATA LOCAL INFILE '/etc/hosts' INTO TABLE conn_t");
} catch (PDOException $e) {
    show($e);
}
$db->exec('DROP TABLE conn_t');
