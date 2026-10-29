<?php
$host = getenv('DBHOST'); $port = (int)getenv('DBPORT');
// Every character set mysqlnd knows (mysqlnd_charset.c), set_charset() and
// get_charset() as it describes them, and escaping in the connection's set:
// a valid multi-byte character is copied whole (its second byte may be a
// \ or a '), the lone start of one is escaped. (WordPress's tests switch the
// connection to big5 and check that its text comes back.)
$m = new mysqli($host, 'wp', 'wp', 'wp', $port);
foreach (['big5', 'latin1', 'koi8r', 'ujis', 'sjis', 'gbk', 'utf8', 'utf8mb4', 'BIG5'] as $cs) {
    var_dump($m->set_charset($cs), $m->character_set_name());
    $c = $m->get_charset();
    echo "$c->charset $c->collation $c->min_length $c->max_length [$c->comment] $c->number $c->state\n";
}
$m->set_charset('big5');
echo bin2hex($m->real_escape_string("a\xa6\x5c'b\xa6")), "\n";
$r = $m->query("SELECT CONVERT( CONVERT( '" . $m->real_escape_string("a\xa6\x40b") . "' USING big5 ) USING big5 ) AS x")->fetch_assoc();
echo bin2hex($r['x']), "\n";
$m->set_charset('gbk'); echo bin2hex($m->real_escape_string("\xbf\x27 or 1=1 \xbf\x5c")), "\n";
$m->set_charset('sjis'); echo bin2hex(mysqli_real_escape_string($m, "\x95\x5c\x81")), "\n";
$m->set_charset('utf8mb4'); echo bin2hex($m->real_escape_string("a\xff'\xe2\x82\xac\"\0")), "\n";
var_dump(mysqli_get_charset($m)->charset);
try { $m->set_charset('nope'); } catch (mysqli_sql_exception $e) { echo $e->getCode(), ' ', $e->getMessage(), "\n"; }
