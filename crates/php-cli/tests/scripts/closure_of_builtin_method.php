<?php
$doc = new DOMDocument();
$make = $doc->createElement(...);
$el = $make('p', 'hi');
var_dump($el->nodeName, $el->textContent, $el->ownerDocument === $doc);
$f = Closure::fromCallable([$doc, 'createTextNode']);
var_dump($f('t')->nodeValue);
$r = new ReflectionFunction($make);
var_dump($r->getName(), $r->getNumberOfParameters(), $r->getClosureThis() === $doc, $r->getClosureScopeClass()->getName());
$it = new ArrayIterator([3, 1, 2]);
$count = $it->count(...);
var_dump($count());
$s = DateTime::createFromFormat(...);
var_dump($s('Y-m-d', '2024-01-02')->format('Y'));
