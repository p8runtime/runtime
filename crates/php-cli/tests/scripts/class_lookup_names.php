<?php
// Only a name that can be a class's is autoloaded ("" and "a-b" never are);
// trait_exists(), enum_exists(); get_parent_class() of an unknown name is a
// TypeError. (Composer's loader reads $class[0]: an empty name warned.)
spl_autoload_register(function ($c) { echo "autoload(", var_export($c, true), ")\n"; });
var_dump(class_exists(''), class_exists('a-b'), interface_exists(''), is_a('stdClass', '', true));
var_dump(class_exists('Valid\Name_1'), class_exists('\Lead'));
enum E {} trait T {} interface I {} class C {}
var_dump(trait_exists('T'), trait_exists('C'), trait_exists('Nope', false), enum_exists('E'), enum_exists('C'));
try { get_parent_class('Nope3'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(get_parent_class('C'));
// ArrayIterator/ArrayObject as Serializable: x:i:FLAGS;STORAGE;m:MEMBERS.
class Filtered extends ArrayIterator { public $kept = 'k'; protected $hidden = 'h'; }
$a = new Filtered([1, 'b' => 2]);
var_dump($s = $a->serialize());
$b = new Filtered;
$b->unserialize(str_replace('"k"', '"K"', $s));
var_dump($b->getArrayCopy(), $b->kept);
var_dump((new ArrayObject(['x'], ArrayObject::ARRAY_AS_PROPS))->serialize());
try { (new ArrayIterator)->unserialize('nonsense'); } catch (UnexpectedValueException $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
