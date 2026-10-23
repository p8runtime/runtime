<?php
class Temp {
    public float $celsius = 0.0;
    public float $fahrenheit {
        get => $this->celsius * 9 / 5 + 32;
        set { $this->celsius = ($value - 32) * 5 / 9; }
    }
    public string $name = 'x' {
        get { echo __METHOD__, "\n"; return strtoupper($this->name); }
        set => trim($value);
    }
    public $log {
        set { echo __FUNCTION__, " ", json_encode($value), "\n"; $this->log = $value; }
    }
    public $readOnly { get => 'ro'; }
    public $writeOnly { set { echo "wrote $value\n"; } }
}
$t = new Temp;
$t->fahrenheit = 212.0;
var_dump($t->celsius, $t->fahrenheit);
$t->name = '  hello  ';
var_dump($t->name);
$r = ($t->log = [1, 2]);
var_dump($r, $t->log);
$t->log += [5 => 5];
$t->log = 10; $t->log++; ++$t->log; $t->log -= 2;
var_dump($t->log);
try { $t->readOnly = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { var_dump($t->writeOnly); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { isset($t->writeOnly); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$t->writeOnly = 'w';
var_dump(isset($t->readOnly), isset($t->celsius), $t->readOnly ?? 'none');
try { unset($t->name); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$t->log = [];
try { $t->log[] = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($t);
var_dump(get_object_vars($t));
echo json_encode($t), "\n";
var_export($t); echo "\n";
class Base { public $v { get { return 'base'; } } public $plain = 'p'; }
class Kid extends Base {
    public $v { get { return parent::$v::get() . '+kid'; } }
    public $plain { get => parent::$plain::get() . '!'; set { parent::$plain::set(strtoupper($value)); } }
}
$k = new Kid; $k->plain = 'q';
var_dump($k->v, $k->plain);
interface HasId { public int $id { get; } }
class Row implements HasId { public int $id = 7; }
var_dump((new Row)->id);
class Loop { public $p { get => $this->other(); } function other() { return $this->p; } }
try { (new Loop)->p; } catch (Error $e) { echo substr($e->getMessage(), 0, 22), "\n"; }
