<?php
// A dynamic property that holds a reference is written through it (#560).
$o = new stdClass;
$o->a = 1;
$o->b = &$o->a;
$o->b = 2;
var_dump($o->a);

$x = 1;
$o->c = &$x;
$o->c = 3;
var_dump($x);

// Compound assignment, by a computed name too.
$s = "a";
$o->d = &$s;
$o->{'d'} .= "b";
$o->d .= "c";
var_dump($s);

// Increment and decrement.
$n = 5;
$o->e = &$n;
++$o->e;
$o->e++;
--$o->e;
var_dump($n);

// Append and a nested key.
$arr = [];
$o->f = &$arr;
$o->f[] = 1;
$o->f['k'] = 2;
var_dump($arr);

// A computed name.
$y = 0;
$name = 'g';
$o->$name = &$y;
$o->$name = 7;
$o->{'g'} = 8;
var_dump($y);

// The reference stays: a later write still reaches the variable.
$x = 10;
var_dump($o->c);
$o->c = 11;
var_dump($x);

// A class allowing no dynamic properties: no deprecation for a write
// to one that exists, and the write goes through.
class Plain {}
$p = new Plain;
$z = 1;
@$p->q = 0;
$p->q = &$z;
$p->q = 4;
var_dump($z);

// unserialize() with R: between dynamic properties, then a write.
$u = unserialize('O:8:"stdClass":2:{s:1:"a";i:1;s:1:"b";R:2;}');
$u->b = 5;
var_dump($u->a);
$u->a = 6;
var_dump($u->b);
$u->b += 10;
var_dump($u->a);
$u->a++;
var_dump($u);

// A reference to an object in a dynamic property: the variable gets the new value.
$w = new stdClass;
$o->h = &$w;
$o->h = 9;
var_dump($w);

// Bound again: the old variable keeps its value, the new one is written.
$r1 = 1;
$r2 = 2;
$o->i = &$r1;
$o->i = &$r2;
$o->i = 3;
var_dump($r1, $r2);

// Unset, then written: a plain property again.
unset($o->i);
$o->i = 4;
var_dump($r2, $o->i);

// foreach by reference over the object's properties.
$f = new stdClass;
$f->a = 1;
$f->b = 2;
foreach ($f as &$v) {}
$f->b = 20;
var_dump($v);
unset($v);
$f->b = 30;
var_dump($f);
