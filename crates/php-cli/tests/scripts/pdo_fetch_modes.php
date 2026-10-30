<?php
class Row {
    public $id;
    public $name;
    public $grp;
    public $args = [];
    public function __construct(...$args) { $this->args = $args; }
}
class Plain {
    public $id;
    public $name;
}
function dump($v) { echo json_encode($v), "\n"; }

$db = new PDO('sqlite::memory:');
$db->exec('CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, grp TEXT)');
$db->exec("INSERT INTO t VALUES (1, 'a', 'x'), (2, 'b', 'x'), (3, 'c', 'y')");
$q = 'SELECT id, name, grp FROM t ORDER BY id';

foreach (['ASSOC', 'NUM', 'BOTH', 'OBJ', 'NAMED'] as $m) {
    echo "== $m\n";
    $st = $db->query($q);
    dump($st->fetch(constant("PDO::FETCH_$m")));
}

echo "== BOTH var_dump\n";
var_dump($db->query($q)->fetch());

echo "== CLASS\n";
try {
    $db->query($q)->fetch(PDO::FETCH_CLASS);
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
$st = $db->query($q);
$st->setFetchMode(PDO::FETCH_CLASS, 'Plain');
dump($st->fetch());
$st->setFetchMode(PDO::FETCH_CLASS, 'Row', ['one', 'two']);
dump($st->fetch());
$st = $db->query($q);
$st->setFetchMode(PDO::FETCH_CLASS | PDO::FETCH_PROPS_LATE, 'Row', ['late']);
dump($st->fetch());
echo "== CLASSTYPE\n";
$st = $db->query('SELECT grp, id, name FROM t ORDER BY id');
$st->setFetchMode(PDO::FETCH_CLASS | PDO::FETCH_CLASSTYPE);
$r = $st->fetch();
var_dump(get_class($r));

echo "== INTO\n";
$st = $db->query($q);
$into = new Plain;
$st->setFetchMode(PDO::FETCH_INTO, $into);
$got = $st->fetch();
var_dump($got === $into, $into->name);

echo "== COLUMN\n";
$st = $db->query($q);
var_dump($st->fetch(PDO::FETCH_COLUMN));
var_dump($st->fetchColumn(1));
$st = $db->query($q);
$st->setFetchMode(PDO::FETCH_COLUMN, 2);
dump($st->fetchAll());

echo "== KEY_PAIR\n";
dump($db->query('SELECT id, name FROM t')->fetchAll(PDO::FETCH_KEY_PAIR));

echo "== GROUP\n";
dump($db->query('SELECT grp, id, name FROM t ORDER BY id')->fetchAll(PDO::FETCH_GROUP | PDO::FETCH_ASSOC));
dump($db->query('SELECT grp, name FROM t ORDER BY id')->fetchAll(PDO::FETCH_GROUP | PDO::FETCH_COLUMN));
echo "== UNIQUE\n";
dump($db->query('SELECT grp, id, name FROM t ORDER BY id')->fetchAll(PDO::FETCH_UNIQUE | PDO::FETCH_ASSOC));

echo "== FUNC\n";
dump($db->query($q)->fetchAll(PDO::FETCH_FUNC, fn($id, $name, $grp) => "$id-$name-$grp"));

echo "== NAMED duplicate columns\n";
dump($db->query('SELECT t.id, t.name, u.name FROM t JOIN t u ON u.id = t.id + 1')->fetchAll(PDO::FETCH_NAMED));
dump($db->query('SELECT t.id, t.name, u.name FROM t JOIN t u ON u.id = t.id + 1')->fetchAll(PDO::FETCH_ASSOC));

echo "== LAZY\n";
$st = $db->query($q);
$row = $st->fetch(PDO::FETCH_LAZY);
var_dump(get_class($row), $row->id, $row->name, $row->queryString);

echo "== fetchObject\n";
$st = $db->query($q);
var_dump(get_class($st->fetchObject()));
var_dump(get_class($st->fetchObject('Plain')));
$o = $st->fetchObject('Row', ['arg']);
dump($o);
var_dump($st->fetchObject());

echo "== default fetch mode\n";
$db->setAttribute(PDO::ATTR_DEFAULT_FETCH_MODE, PDO::FETCH_NUM);
dump($db->query($q)->fetch());
$db->setAttribute(PDO::ATTR_DEFAULT_FETCH_MODE, PDO::FETCH_ASSOC);
dump($db->query($q)->fetchAll());
var_dump($db->getAttribute(PDO::ATTR_DEFAULT_FETCH_MODE));

echo "== BOUND\n";
$st = $db->prepare($q);
$st->execute();
$st->bindColumn(1, $id, PDO::PARAM_INT);
$st->bindColumn('name', $name);
$st->bindColumn(3, $grp);
while ($st->fetch(PDO::FETCH_BOUND)) {
    var_dump($id, $name, $grp);
}

echo "== fetch after end\n";
$st = $db->query('SELECT 1');
var_dump($st->fetch(PDO::FETCH_NUM), $st->fetch(PDO::FETCH_NUM));
var_dump($st->fetchAll());
