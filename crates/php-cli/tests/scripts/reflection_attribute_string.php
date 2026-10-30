<?php
#[Attribute] class A { function __construct(...$a) {} }
#[A] class C1 {}
#[A(1, 'x', name: [1, 2], o: PHP_INT_MAX, e: null, f: 1.5, b: true)] class C2 {}
#[A, Undefined('q')] class C3 {}
foreach (['C1', 'C2', 'C3'] as $c) foreach ((new ReflectionClass($c))->getAttributes() as $a) echo "[", $a, "]\n";
