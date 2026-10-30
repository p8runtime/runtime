<?php
// Reflection on builtin interface methods (abstract, no code): from the stubs,
// with tentative return types apart from real ones.
foreach ([['Countable', 'count'], ['IteratorAggregate', 'getIterator'], ['ArrayAccess', 'offsetGet'], ['JsonSerializable', 'jsonSerialize']] as [$c, $m]) {
    $r = new ReflectionMethod($c, $m);
    var_dump($r->getName(), $r->returnsReference(), $r->isAbstract(), $r->getNumberOfParameters(), (string) $r->getReturnType(), $r->getStaticVariables());
}
interface Mine extends Countable {}
var_dump((new ReflectionMethod('Mine', 'count'))->returnsReference());
foreach ([['Countable', 'count'], ['ArrayIterator', 'current'], ['Exception', 'getMessage']] as [$c, $m]) {
    $r = new ReflectionMethod($c, $m);
    var_dump($r->hasReturnType(), $r->hasTentativeReturnType(), (string) $r->getTentativeReturnType(), (string) $r->getReturnType());
}
