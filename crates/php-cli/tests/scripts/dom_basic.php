<?php
$d = new DOMDocument();
$d->loadXML('<root a="1"><item>one</item><item>two</item></root>');
echo $d->documentElement->nodeName, " ", $d->documentElement->getAttribute('a'), "\n";
foreach ($d->getElementsByTagName('item') as $i) echo $i->textContent, ",";
$list = $d->documentElement->childNodes;
echo "\n", $list->length, " ", $list[1]->textContent, " ", count($list), "\n";
var_dump(isset($list[1]), isset($list[2]), $d->documentElement->attributes['a']->value);
$e = $d->createElement('new', 'x &amp; y');
$d->documentElement->appendChild($e);
var_dump($d->documentElement->lastChild === $e, $e->parentNode === $d->documentElement, $e->ownerDocument === $d);
$d->createElement('bad', 'x & y');
$d->formatOutput = true;
echo $d->saveXML();
echo $d->saveXML($e), "\n";
try { $d->createElement('1bad'); } catch (DOMException $ex) { echo get_class($ex), ': ', $ex->getMessage(), ' ', $ex->getCode(), "\n"; }
try { $d->documentElement->appendChild($d->documentElement); } catch (DOMException $ex) { echo $ex->getMessage(), "\n"; }
$t = new DOMText('orphan');
var_dump($t->ownerDocument, $t->isConnected);
$d->documentElement->appendChild($t);
var_dump($t->isConnected, $t->ownerDocument === $d);
$frag = $d->createDocumentFragment();
$frag->appendXML('<f1/><f2>z</f2>');
$d->documentElement->replaceChildren($frag);
echo $d->saveXML($d->documentElement), "\n";
$d->documentElement->firstChild->after('text', $d->createComment('c'));
echo $d->saveXML($d->documentElement), "\n";
var_dump($d->loadXML('<a><b></a>'));
var_dump(method_exists($d, '$nodeName::get'), in_array('$nodeName::get', get_class_methods($d)));
