<?php
// DOMDocument::schemaValidate[Source]: schemas that cannot be used, and the warnings PHP gives
// when libxml's internal errors are off.
function show() {
    foreach (libxml_get_errors() as $e) {
        echo "  L{$e->level} C{$e->code} col{$e->column} line{$e->line} ", rtrim($e->message), "\n";
    }
    libxml_clear_errors();
    echo "--\n";
}
$xs = '<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">';
$schemas = [
    'string that is not a schema',
    '<root/>',
    $xs . '<xs:element name="a" type="nope"/></xs:schema>',
    $xs . '<xs:element name="a" type="xs:string" bar="1"/></xs:schema>',
    $xs . '<xs:element/></xs:schema>',
    $xs . '<xs:element name="a" minOccurs="2"/></xs:schema>',
    $xs . '<xs:element name="a" substitutionGroup="b"/><xs:element name="b" type="xs:string"/></xs:schema>',
    $xs . '<xs:element name="a" type="xs:int"/><xs:element name="b" type="xs:string" substitutionGroup="a"/></xs:schema>',
    $xs . '<xs:simpleType name="s"><xs:restriction base="nope:t"/></xs:simpleType></xs:schema>',
    $xs . '<xs:element name="r"><xs:complexType><xs:group ref="g"/></xs:complexType></xs:element></xs:schema>',
    $xs . '<xs:element name="r"><xs:complexType><xs:attributeGroup ref="g"/></xs:complexType></xs:element></xs:schema>',
    $xs . '<xs:element name="r"><xs:complexType><xs:sequence><xs:element ref="nope"/></xs:sequence></xs:complexType></xs:element></xs:schema>',
];
libxml_use_internal_errors(true);
$d = new DOMDocument;
$d->loadXML('<a/>');
foreach ($schemas as $s) {
    echo $s, "\n";
    var_dump($d->schemaValidateSource($s));
    show();
}
// More schema errors (a target namespace, other component kinds).
$xs2 = '<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:t="urn:t" targetNamespace="urn:t">';
$schemas2 = [
    '<xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a" type="nope"/></xs:sequence></xs:complexType></xs:element>',
    '<xs:element name="r"><xs:complexType><xs:attribute name="a" type="nope"/></xs:complexType></xs:element>',
    '<xs:element name="r"><xs:complexType><xs:attribute ref="nope"/></xs:complexType></xs:element>',
    '<xs:attribute name="g" type="nope"/>',
    '<xs:complexType name="c"><xs:complexContent><xs:extension base="nope"/></xs:complexContent></xs:complexType>',
    '<xs:complexType name="c"><xs:simpleContent><xs:extension base="xs:nope"/></xs:simpleContent></xs:complexType>',
    '<xs:simpleType name="s"><xs:restriction base="xs:nope"/></xs:simpleType>',
    '<xs:simpleType name="s"><xs:list itemType="xs:nope"/></xs:simpleType>',
    '<xs:simpleType name="s"><xs:union memberTypes="xs:int xs:nope t:nope"/></xs:simpleType>',
    '<xs:element name="r" type="t:nope"/>',
    '<xs:element name="r" type="t:nope"/><xs:element name="q" type="x:y"/>',
    '<xs:element name="r" substitutionGroup="t:nope"/>',
    '<xs:element name="r"><xs:complexType><xs:sequence><xs:element ref="t:nope"/></xs:sequence></xs:complexType></xs:element>',
    '<xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a" type="nope"/><xs:element name="b" type="nope2"/></xs:sequence></xs:complexType></xs:element>',
    '<xs:element name="r"><xs:foo/></xs:element>',
    '<xs:element name="r"><xs:complexType><xs:foo/></xs:complexType></xs:element>',
    '<xs:foo/>',
    '<xs:element name="r" default="a" fixed="b"/>',
    '<xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a" minOccurs="3" maxOccurs="2"/></xs:sequence></xs:complexType></xs:element>',
    '<xs:element name="r"/><xs:element name="r"/>',
    '<xs:complexType name="c"/><xs:simpleType name="c"><xs:restriction base="xs:int"/></xs:simpleType>',
    '<xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a"/><xs:element name="a" type="xs:int"/></xs:sequence></xs:complexType></xs:element>',
    '<xs:element name="r"><xs:complexType><xs:all><xs:element name="a" maxOccurs="2"/></xs:all></xs:complexType></xs:element>',
    '<xs:element name="r"><xs:complexType><xs:attribute name="a"/><xs:attribute name="a"/></xs:complexType></xs:element>',
];
foreach ($schemas2 as $s) {
    echo $s, "\n";
    $d->schemaValidateSource($xs2 . $s . '</xs:schema>');
    show();
}

var_dump($d->schemaValidate('/nonexistent.xsd'));
show();
$file = sys_get_temp_dir() . '/dom_schema_errors_junk.xsd';
file_put_contents($file, 'junk');
var_dump($d->schemaValidate($file));
foreach (libxml_get_errors() as $e) {
    echo "  L{$e->level} C{$e->code} ", str_replace($file, 'FILE', rtrim($e->message)), "\n";
}
libxml_clear_errors();
unlink($file);

// The same without internal errors: each a warning.
libxml_use_internal_errors(false);
var_dump($d->schemaValidateSource('string that is not a schema'));
var_dump($d->schemaValidateSource('<root/>'));
var_dump($d->schemaValidateSource($xs . '<xs:element name="a" type="nope"/></xs:schema>'));
var_dump($d->schemaValidateSource($xs . '<xs:element name="r"/></xs:schema>'));
var_dump($d->schemaValidate('/nonexistent.xsd'));
$e = new DOMDocument;
var_dump($e->schemaValidateSource($xs . '<xs:element name="r"/></xs:schema>'));
foreach (['', "a\0b"] as $name) {
    foreach (['schemaValidate', 'schemaValidateSource'] as $m) {
        try {
            $d->$m($name);
        } catch (ValueError $ex) {
            echo $ex->getMessage(), "\n";
        }
    }
}
