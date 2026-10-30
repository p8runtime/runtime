<?php
function show(PDOException $e) {
    echo get_class($e), ' | ', $e->getMessage(), ' | ', var_export($e->getCode(), true), ' | ', json_encode($e->errorInfo), "\n";
}

$db = new PDO('sqlite::memory:');
$db->exec('CREATE TABLE u (id INTEGER PRIMARY KEY, email TEXT UNIQUE NOT NULL)');
$db->exec("INSERT INTO u (email) VALUES ('a@x')");

echo "== exception mode (default)\n";
try { $db->exec('INSERT INTO nope VALUES (1)'); } catch (PDOException $e) { show($e); }
try { $db->query('SELECT * FROM nope'); } catch (PDOException $e) { show($e); }
try { $db->prepare('SELECT * FROM nope'); } catch (PDOException $e) { show($e); }
try { $db->exec("INSERT INTO u (email) VALUES ('a@x')"); } catch (PDOException $e) { show($e); }
try { $db->exec("INSERT INTO u (email) VALUES (NULL)"); } catch (PDOException $e) { show($e); }
$st = $db->prepare('INSERT INTO u (email) VALUES (?)');
try { $st->execute(['a@x']); } catch (PDOException $e) { show($e); }
var_dump($st->errorCode());
var_dump($st->errorInfo()[0], $st->errorInfo()[1]);

echo "== bad parameter number\n";
$st = $db->prepare('SELECT * FROM u WHERE id = ?');
try { $st->execute([1, 2]); } catch (PDOException $e) { show($e); }
try { $st->bindValue(0, 1); } catch (ValueError $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { $st->bindValue(':x', 1); var_dump($st->execute()); } catch (PDOException $e) { show($e); }

echo "== warning mode\n";
$db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_WARNING);
var_dump($db->exec('INSERT INTO nope VALUES (1)'));
var_dump($db->errorCode());
var_dump($db->query('SELECT * FROM nope'));
$st = $db->prepare('INSERT INTO u (email) VALUES (?)');
var_dump($st->execute(['a@x']));
var_dump($st->errorCode());

echo "== silent mode\n";
$db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_SILENT);
var_dump($db->exec('INSERT INTO nope VALUES (1)'));
var_dump($db->errorCode());
var_dump($db->errorInfo()[0], $db->errorInfo()[1]);
var_dump($db->query('SELECT * FROM nope'));
var_dump($db->errorCode());

echo "== setAttribute checks\n";
try { $db->setAttribute(PDO::ATTR_ERRMODE, 99); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $db->setAttribute(PDO::ATTR_CASE, 99); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $db->setAttribute(PDO::ATTR_ERRMODE, 'x'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION);

echo "== connection errors\n";
try { new PDO('nodriver:x'); } catch (PDOException $e) { show($e); }
try { new PDO('nocolon'); } catch (PDOException $e) { show($e); }
try { new PDO('sqlite:/nonexistent-dir/x.db'); } catch (PDOException $e) { show($e); }

echo "== misc\n";
try { $db->commit(); } catch (PDOException $e) { show($e); }
try { $db->rollBack(); } catch (PDOException $e) { show($e); }
$db->beginTransaction();
try { $db->beginTransaction(); } catch (PDOException $e) { show($e); }
$db->rollBack();
try { $db->prepare(''); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $db->exec(''); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { var_dump($db->getAttribute(PDO::ATTR_AUTOCOMMIT)); } catch (PDOException $e) { show($e); }
