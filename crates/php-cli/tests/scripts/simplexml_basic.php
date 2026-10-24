<?php
$xml = simplexml_load_string('<?xml version="1.0"?>
<library name="main">
  <book id="1" lang="en"><title>Dune</title><price>9.5</price></book>
  <book id="2"><title>Emma</title><price>4</price></book>
  <!-- note -->
  <empty/>
</library>');
echo $xml['name'], " ", $xml->getName(), " ", count($xml->book), "\n";
foreach ($xml->book as $b) echo $b['id'], ":", $b->title, " ";
echo "\n", $xml->book[1]->title, " ", (float)$xml->book[0]->price + 1, " ", $xml->book->price * 2, "\n";
var_dump(isset($xml->book), isset($xml->nothing), isset($xml->book[1]), isset($xml->book[5]), isset($xml['name']), isset($xml['x']));
var_dump((bool)$xml->empty, (bool)$xml->nothing, (string)$xml->nothing);
foreach ($xml->children() as $k => $v) echo $k, " ";
echo "\n";
foreach ($xml->book[0]->attributes() as $k => $v) echo "$k=$v ";
echo "\n";
$xml->book[0]->title = 'Dune Messiah';
$xml->book[1]['id'] = 'two';
$xml['added'] = 'yes';
$c = $xml->addChild('book');
$c->addAttribute('id', '3');
$c->addChild('title', 'New & Shiny');
unset($xml->empty);
echo $xml->asXML();
echo $xml->book[0]->asXML(), "\n";
print_r($xml->book[1]);
print_r($xml->book[0]);
echo "\n";
var_dump(json_encode($xml->book[1]));
var_dump((array)$xml->book[0]->title);
$r = simplexml_load_string('<a><b>');
var_dump($r);
try { new SimpleXMLElement('nope'); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$ns = simplexml_load_string('<r xmlns:x="urn:x"><x:a>1</x:a><b>2</b></r>');
var_dump($ns->getNamespaces(true), $ns->getDocNamespaces(), count($ns->children('urn:x')), (string)$ns->children('x', true)->a);
