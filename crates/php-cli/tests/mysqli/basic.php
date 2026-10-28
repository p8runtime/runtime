<?php
$host = getenv('DBHOST'); $port = (int)getenv('DBPORT');
// strict mode (the default): a bad password throws
try { new mysqli($host, 'wp', 'nope', 'wp', $port); } catch (mysqli_sql_exception $e) {
    echo get_class($e), ' ', $e->getCode(), ' ', $e->getSqlState(), ' ', preg_replace("/'@'[^']*'/", "'@'X'", $e->getMessage()), "\n";
}
var_dump(mysqli_connect_errno());
mysqli_report(MYSQLI_REPORT_OFF);
$m = mysqli_init();
var_dump($m->connect_errno, $m->errno, $m->error);
var_dump(@mysqli_real_connect($m, $host, 'wp', 'wp', null, $port));
var_dump($m->connect_errno, $m->connect_error, mysqli_connect_error());
var_dump(mysqli_select_db($m, 'wp'), mysqli_character_set_name($m), $m->set_charset('latin1'), $m->character_set_name(), mysqli_set_charset($m, 'utf8mb4'));
var_dump(mysqli_query($m, 'DROP TABLE IF EXISTS t'));
var_dump(mysqli_query($m, 'CREATE TABLE t (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(20), score DECIMAL(5,2), note TEXT NULL)'));
var_dump(mysqli_query($m, "INSERT INTO t (name, score, note) VALUES ('a', 1.5, NULL), ('b''s', 20, 'x'), ('" . mysqli_real_escape_string($m, "c\\'\"\n") . "', 3, '')"));
var_dump(mysqli_affected_rows($m), mysqli_insert_id($m), $m->insert_id, $m->affected_rows, $m->info, mysqli_info($m), $m->field_count);
var_dump(mysqli_query($m, 'UPDATE t SET score = score + 1 WHERE id > 1'), $m->affected_rows, $m->info);
$r = mysqli_query($m, 'SELECT * FROM t ORDER BY id');
var_dump($r instanceof mysqli_result, mysqli_num_rows($r), $r->num_rows, mysqli_num_fields($r), $r->field_count, mysqli_field_count($m), $m->affected_rows);
var_dump(mysqli_fetch_assoc($r), $r->fetch_row(), mysqli_fetch_lengths($r), $r->fetch_array(), $r->fetch_array(MYSQLI_ASSOC));
var_dump($r->data_seek(1)); var_dump(get_object_vars(mysqli_fetch_object($r)), $r->data_seek(9));
$r->data_seek(0);
var_dump($r->fetch_all(MYSQLI_ASSOC));
$f = mysqli_fetch_field($r);
foreach (['name', 'orgname', 'table', 'orgtable', 'def', 'db', 'max_length', 'length', 'charsetnr', 'flags', 'type', 'decimals'] as $k) echo "$k=", var_export($f->$k, true), " ";
echo "\n";
foreach ($r->fetch_fields() as $f) echo $f->name, ':', $f->type, ':', $f->length, ' ';
echo "\n";
var_dump(mysqli_field_tell($r), $r->current_field);
$r->data_seek(0);
foreach ($r as $i => $row) echo $i, '=', json_encode($row), "\n";
mysqli_free_result($r);
try { $r->fetch_row(); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
var_dump(mysqli_query($m, 'SELEC oops'), mysqli_errno($m), mysqli_error($m), mysqli_sqlstate($m), $m->errno, $m->sqlstate, count($m->error_list), $m->affected_rows);
mysqli_report(MYSQLI_REPORT_ERROR);
var_dump(mysqli_query($m, 'SELECT * FROM missing'));
mysqli_report(MYSQLI_REPORT_ERROR | MYSQLI_REPORT_STRICT);
try { $m->query('SELECT * FROM missing'); } catch (mysqli_sql_exception $e) { echo $e->getCode(), ' ', $e->getSqlState(), ' ', $e->getMessage(), "\n"; }
var_dump($m->query('SELECT 1 AS one, NULL AS n, "x" AS `1`')->fetch_assoc());
var_dump($m->query("SELECT COUNT(*) c FROM t")->fetch_column(), $m->query("SELECT name FROM t WHERE id = 99")->fetch_assoc());
var_dump(mysqli_more_results($m), mysqli_next_result($m), mysqli_ping($m), $m->begin_transaction(), $m->rollback(), mysqli_autocommit($m, true));
var_dump(is_int($m->thread_id), is_string($m->server_info), $m->server_version > 0, $m->protocol_version, mysqli_get_client_info(), mysqli_get_client_version(), $m->client_info);
var_dump(mysqli_close($m));
try { mysqli_query($m, 'SELECT 1'); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
