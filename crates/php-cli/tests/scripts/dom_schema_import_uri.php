<?php
// A schemaLocation is a URI: a file: URL with percent-escapes (as Symfony's
// XliffUtils writes it), or a path relative to the importing schema.
$dir = sys_get_temp_dir() . '/p8 schema ' . getmypid();
@mkdir($dir);
file_put_contents("$dir/xml.xsd", '<?xml version="1.0"?><xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="http://www.w3.org/XML/1998/namespace"><xs:attribute name="lang" type="xs:language"/></xs:schema>');
$url = 'file://' . implode('/', array_map('rawurlencode', explode('/', "$dir/xml.xsd")));
$schema = '<?xml version="1.0"?><xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:t" xmlns="urn:t" elementFormDefault="qualified">'
    . '<xs:import namespace="http://www.w3.org/XML/1998/namespace" schemaLocation="URL"/>'
    . '<xs:element name="note"><xs:complexType><xs:simpleContent><xs:extension base="xs:string"><xs:attribute ref="xml:lang"/></xs:extension></xs:simpleContent></xs:complexType></xs:element></xs:schema>';
$doc = new DOMDocument();
$doc->loadXML('<note xmlns="urn:t" xml:lang="en">hi</note>');
var_dump($doc->schemaValidateSource(str_replace('URL', $url, $schema)));
file_put_contents("$dir/main.xsd", str_replace('URL', 'xml.xsd', $schema));
var_dump($doc->schemaValidate("$dir/main.xsd"));
$bad = new DOMDocument();
$bad->loadXML('<note xmlns="urn:t" xml:lang="not a language!">hi</note>');
libxml_use_internal_errors(true);
var_dump($bad->schemaValidateSource(str_replace('URL', $url, $schema)));
foreach (libxml_get_errors() as $e) { echo trim($e->message), "\n"; }
unlink("$dir/xml.xsd"); unlink("$dir/main.xsd"); rmdir($dir);
