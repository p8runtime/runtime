<?php
$db = new PDO('sqlite::memory:');
$db->exec('CREATE TABLE it (id INTEGER PRIMARY KEY, v TEXT)');
$db->exec("INSERT INTO it (v) VALUES ('a'), ('b'), ('c'), ('d')");

echo "== foreach\n";
$st = $db->query('SELECT * FROM it ORDER BY id');
foreach ($st as $k => $row) {
    echo $k, ' ', json_encode($row), "\n";
}
var_dump($st->fetch());

echo "== prepared, executed twice\n";
$st = $db->prepare('SELECT v FROM it WHERE id > :min ORDER BY id');
$st->setFetchMode(PDO::FETCH_COLUMN, 0);
$st->execute([':min' => 2]);
foreach ($st as $v) echo $v;
echo "\n";
$st->execute([':min' => 0]);
foreach ($st as $v) echo $v;
echo "\n";

echo "== break and continue\n";
$st = $db->query('SELECT v FROM it ORDER BY id');
foreach ($st as $row) {
    echo $row['v'];
    if ($row['v'] === 'b') break;
}
echo "|", $st->fetchColumn(), "\n";

echo "== iterator_to_array and getIterator\n";
$st = $db->query('SELECT v FROM it ORDER BY id', PDO::FETCH_NUM);
echo json_encode(iterator_to_array($st)), "\n";
$st = $db->query('SELECT v FROM it ORDER BY id', PDO::FETCH_NUM);
$it = $st->getIterator();
var_dump($it instanceof Iterator, $it instanceof Traversable);
var_dump($st instanceof IteratorAggregate, $st instanceof Traversable);

echo "== fetch modes in foreach\n";
$st = $db->query('SELECT id, v FROM it ORDER BY id', PDO::FETCH_KEY_PAIR);
echo json_encode(iterator_to_array($st)), "\n";
$st = $db->query('SELECT id, v FROM it ORDER BY id LIMIT 2', PDO::FETCH_OBJ);
foreach ($st as $o) echo get_class($o), ':', $o->v, ' ';
echo "\n";

echo "== nested loops on separate statements\n";
foreach ($db->query('SELECT id FROM it WHERE id < 3 ORDER BY id') as $a) {
    foreach ($db->query('SELECT v FROM it WHERE id > ' . (int) $a['id'] . ' ORDER BY id LIMIT 2') as $b) {
        echo $a['id'], $b['v'], ' ';
    }
}
echo "\n";

echo "== statement class\n";
class MyStmt extends PDOStatement {
    public $tag;
    protected function __construct($tag = 'none') { $this->tag = $tag; }
}
$db->setAttribute(PDO::ATTR_STATEMENT_CLASS, ['MyStmt', ['custom']]);
$st = $db->query('SELECT 1');
var_dump(get_class($st), $st->tag, $st->queryString);
$st = $db->prepare('SELECT 2');
var_dump(get_class($st), $st->tag);
var_dump($db->getAttribute(PDO::ATTR_STATEMENT_CLASS));
try { $db->setAttribute(PDO::ATTR_STATEMENT_CLASS, 'PDOStatement'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $db->setAttribute(PDO::ATTR_STATEMENT_CLASS, ['NoSuchClass']); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $db->setAttribute(PDO::ATTR_STATEMENT_CLASS, ['stdClass']); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }

echo "== subclass of PDO\n";
class MyPDO extends PDO {
    public function __construct($dsn) { parent::__construct($dsn); $this->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION); }
    public function hello() { return 'hi ' . $this->getAttribute(PDO::ATTR_DRIVER_NAME); }
}
$m = new MyPDO('sqlite::memory:');
var_dump($m->hello(), $m instanceof PDO);
var_dump($m->query('SELECT 5')->fetchColumn());
try { clone $m; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { new PDORow; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$s = new Pdo\Sqlite('sqlite::memory:');
var_dump($s instanceof PDO, get_class($s->query('SELECT 1')));
try { new Pdo\Sqlite('mysql:host=x'); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { PDO::connect('sqlite::memory:')->nothing(); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
