<?php
// DOMDocument::registerNodeClass: node objects made with the registered class
// (it holds across loads); the argument checks (Parsoid registers its DOM classes).
class MyEl extends DOMElement { function hi() { return 'hi ' . $this->nodeName; } }
class MyText extends DOMText {}
class MyDoc extends DOMDocument {}
class Other {}
$d = new DOMDocument;
var_dump($d->registerNodeClass('DOMElement', 'MyEl'));
$d->loadXML('<r><a>t</a></r>');
var_dump(get_class($d->documentElement), $d->documentElement->hi(), get_class($d->documentElement->firstChild));
$e = $d->createElement('x'); var_dump(get_class($e));
var_dump($d->registerNodeClass('DOMText', 'MyText'), get_class($d->createTextNode('z')), get_class($d->documentElement->firstChild->firstChild));
var_dump($d->registerNodeClass('DOMElement', null), get_class($d->createElement('y')));
foreach ([['DOMElement', 'Other'], ['Other', 'MyEl'], ['DOMElement', 'Nope'], ['DOMText', 'MyEl']] as [$a, $b]) {
    try { $d->registerNodeClass($a, $b); echo "ok\n"; } catch (Throwable $t) { echo get_class($t), ': ', $t->getMessage(), "\n"; }
}
var_dump($d->registerNodeClass('DOMDocument', 'MyDoc'), get_class($d->documentElement->ownerDocument));
$p = new DOMDocument; $p->registerNodeClass('DOMElement', 'MyEl'); $p->loadXML('<r/>'); $c = $p->documentElement; unset($c); var_dump(get_class($p->documentElement));
