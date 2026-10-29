<?php
function v($xml, $xsd, $flags = 0) {
    libxml_use_internal_errors(true);
    libxml_clear_errors();
    $d = new DOMDocument; $d->loadXML($xml);
    $r = $d->schemaValidateSource($xsd, $flags);
    echo $r ? "VALID\n" : "INVALID\n";
    foreach (libxml_get_errors() as $e) {
        echo "  L{$e->level} C{$e->code} line{$e->line} ", rtrim($e->message), "\n";
    }
    if ($flags) echo $d->saveXML();
}
function t($title, $xsd, $cases, $flags = 0) {
    echo "##### $title\n";
    $xsd = '<?xml version="1.0"?><xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" ' . $xsd;
    foreach ($cases as $c) { echo "== $c\n"; v($c, $xsd, $flags); }
}
t('all', '><xs:element name="r"><xs:complexType><xs:all><xs:element name="a"/><xs:element name="b" minOccurs="0"/><xs:element name="c"/></xs:all></xs:complexType></xs:element></xs:schema>', [
 '<r><a/><c/></r>', '<r><c/><b/><a/></r>', '<r><a/></r>', '<r/>', '<r><a/><a/><c/></r>', '<r><a/><x/><c/></r>', '<r><b/></r>', '<r><a/><c/><d/><b/></r>',
]);
t('all optional', '><xs:element name="r"><xs:complexType><xs:all minOccurs="0"><xs:element name="a"/></xs:all></xs:complexType></xs:element></xs:schema>', ['<r/>', '<r><a/></r>', '<r><b/></r>']);
t('sequence-bad', '><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a"/><xs:element name="b"/></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 '<r><b/><a/></r>', '<r><a/><b/><b/><c/></r>', '<r><a/><a/></r>', '<r></r>', '<r><a/></r>',
]);
t('star', '><xs:element name="r"><xs:complexType><xs:sequence><xs:choice minOccurs="0" maxOccurs="unbounded"><xs:element name="a"/><xs:element name="b"/></xs:choice><xs:element name="z"/></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 '<r><a/><b/><a/><z/></r>', '<r><a/><c/></r>', '<r><a/></r>', '<r><z/><a/></r>',
]);
t('counted', '><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a" minOccurs="2" maxOccurs="3"/><xs:element name="b" minOccurs="0"/></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 '<r><a/></r>', '<r><a/><a/></r>', '<r><a/><a/><a/><a/></r>', '<r><a/><a/><b/><b/></r>',
]);
t('empty', '><xs:element name="r"><xs:complexType><xs:attribute name="x"/></xs:complexType></xs:element></xs:schema>', [
 '<r/>', '<r>text</r>', '<r> </r>', '<r><a/></r>', '<r x="1" y="2"/>', '<!--c--><r><!--c--></r>',
]);
t('mixed', '><xs:element name="r"><xs:complexType mixed="true"><xs:sequence><xs:element name="a" minOccurs="0"/></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 '<r>text<a/>more</r>', '<r><b/></r>', '<r>x</r>',
]);
t('simplecontent', '><xs:element name="r"><xs:complexType><xs:simpleContent><xs:extension base="xs:int"><xs:attribute name="u" use="required"/></xs:extension></xs:simpleContent></xs:complexType></xs:element></xs:schema>', [
 '<r u="1">5</r>', '<r>5</r>', '<r u="1">x</r>', '<r u="1"><a/></r>', '<r u="1"/>',
]);
t('nested errors', '><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a" maxOccurs="unbounded"><xs:complexType><xs:sequence><xs:element name="b" type="xs:int"/></xs:sequence><xs:attribute name="i" type="xs:int" use="required"/></xs:complexType></xs:element></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 "<r>\n<a i=\"x\">\n<b>q</b>\n</a>\n<a>\n<c/>\n<b>1</b>\n</a>\n</r>",
 '<r><a><b>1</b><b>2</b></a></r>',
 '<r><a i="1"><b>1</b></a><a><b>1</b><x/><y/></a><z/></r>',
]);
t('attr order', '><xs:element name="r"><xs:complexType><xs:attribute name="a" type="xs:int" use="required"/><xs:attribute name="b" type="xs:int"/><xs:attribute name="c" type="xs:int" use="required"/></xs:complexType></xs:element></xs:schema>', [
 '<r b="x" z="1"/>', '<r z="1" b="x" c="y"/>', '<r z="1" y="2"/>',
]);
t('ns', ' targetNamespace="urn:t" xmlns="urn:t" elementFormDefault="qualified"><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a" type="xs:int"/><xs:element name="b" form="unqualified" minOccurs="0"/></xs:sequence><xs:attribute name="q" form="qualified"/><xs:attribute name="u"/></xs:complexType></xs:element></xs:schema>', [
 '<r xmlns="urn:t"><a>1</a></r>', '<r xmlns="urn:t"><a>x</a><b/></r>', '<r><a>1</a></r>', '<t:r xmlns:t="urn:t"><t:a>1</t:a><b/></t:r>',
 '<r xmlns="urn:t" xmlns:t="urn:t" t:q="1" u="2" t:u="3"><a>1</a><c/></r>', '<r xmlns="urn:t"><b/></r>', '<x:r xmlns:x="urn:other"/>',
]);
t('subst', '><xs:element name="head" abstract="true" type="xs:string"/><xs:element name="m1" substitutionGroup="head"/><xs:element name="m2" substitutionGroup="head" type="xs:int"/><xs:element name="r"><xs:complexType><xs:sequence><xs:element ref="head" maxOccurs="2"/><xs:element name="z"/></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 '<r><m1>a</m1><m2>3</m2><z/></r>', '<r><m2>x</m2><z/></r>', '<r><head/><z/></r>', '<r><z/></r>', '<r><m1/></r>', '<r><m1/><m1/><m1/></r>',
]);
t('substnoabs', '><xs:element name="head" type="xs:string"/><xs:element name="m1" substitutionGroup="head"/><xs:element name="r"><xs:complexType><xs:sequence><xs:element ref="head"/><xs:element name="z"/></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 '<r><m1/></r>', '<r><q/></r>', '<head/>', '<m1/>',
]);
t('abstract elem root', '><xs:element name="r" abstract="true" type="xs:string"/></xs:schema>', ['<r/>']);
t('any', '><xs:element name="r"><xs:complexType><xs:sequence><xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/><xs:element name="z" minOccurs="0"/></xs:sequence><xs:anyAttribute namespace="urn:o"/></xs:complexType></xs:element><xs:element name="g" type="xs:int"/></xs:schema>', [
 '<r><a:x xmlns:a="urn:a">1</a:x><z/></r>', '<r><x/></r>', '<r xmlns:o="urn:o" o:k="1" k="2"/>', '<r><g xmlns="">5</g></r>',
]);
t('any strict', '><xs:element name="r"><xs:complexType><xs:sequence><xs:any processContents="strict"/></xs:sequence></xs:complexType></xs:element><xs:element name="g" type="xs:int"/></xs:schema>', [
 '<r><g>x</g></r>', '<r><h/></r>', '<r/>',
]);
t('any skip any', '><xs:element name="r"><xs:complexType><xs:sequence><xs:any processContents="skip" maxOccurs="2"/></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 '<r><a/><b/><c/></r>', '<r/>',
]);
t('nillable', '><xs:element name="r" nillable="true" type="xs:int"/></xs:schema>', [
 '<r xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:nil="true"/>', '<r xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:nil="true">5</r>', '<r xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:nil="false">5</r>',
]);
t('not nillable', '><xs:element name="r" type="xs:int"/></xs:schema>', [
 '<r xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:nil="true"/>',
]);
t('xsi:type', '><xs:complexType name="b"><xs:sequence><xs:element name="x"/></xs:sequence></xs:complexType><xs:complexType name="d"><xs:complexContent><xs:extension base="b"><xs:sequence><xs:element name="y"/></xs:sequence></xs:extension></xs:complexContent></xs:complexType><xs:complexType name="o"/><xs:element name="r" type="b"/></xs:schema>', [
 '<r xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="d"><x/><y/></r>', '<r xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="o"/>', '<r xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="nope"/>', '<r><x/><y/></r>',
]);
t('ext', '><xs:complexType name="b"><xs:sequence><xs:element name="x"/></xs:sequence><xs:attribute name="p"/></xs:complexType><xs:complexType name="d"><xs:complexContent><xs:extension base="b"><xs:sequence><xs:element name="y" minOccurs="0"/></xs:sequence><xs:attribute name="q" use="required"/></xs:extension></xs:complexContent></xs:complexType><xs:element name="r" type="d"/></xs:schema>', [
 '<r q="1" p="2"><x/><y/></r>', '<r><x/></r>', '<r q="1"><y/></r>', '<r q="1"><x/><y/><z/></r>',
]);
t('restr', '><xs:complexType name="b"><xs:sequence><xs:element name="x"/><xs:element name="y" minOccurs="0"/></xs:sequence><xs:attribute name="p"/></xs:complexType><xs:complexType name="d"><xs:complexContent><xs:restriction base="b"><xs:sequence><xs:element name="x"/></xs:sequence></xs:restriction></xs:complexContent></xs:complexType><xs:element name="r" type="d"/></xs:schema>', [
 '<r p="1"><x/></r>', '<r><x/><y/></r>',
]);
t('group refs', '><xs:group name="g"><xs:choice><xs:element name="a"/><xs:element name="b"/></xs:choice></xs:group><xs:attributeGroup name="ag"><xs:attribute name="k" type="xs:int"/></xs:attributeGroup><xs:element name="r"><xs:complexType><xs:sequence><xs:group ref="g" maxOccurs="2"/><xs:element name="z"/></xs:sequence><xs:attributeGroup ref="ag"/></xs:complexType></xs:element></xs:schema>', [
 '<r k="x"><a/><b/><z/></r>', '<r><a/><b/><a/><z/></r>', '<r><z/></r>',
]);
t('default fixed el', '><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="d" type="xs:int" default="7" minOccurs="0"/><xs:element name="f" type="xs:int" fixed="3" minOccurs="0"/></xs:sequence><xs:attribute name="a" default="A"/><xs:attribute name="b" fixed="B"/></xs:complexType></xs:element></xs:schema>', [
 '<r><d/><f>4</f></r>', '<r><d></d><f/></r>', '<r a="x" b="B"><f>3</f></r>',
]);
t('default create', '><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="d" type="xs:int" default="7" minOccurs="0"/><xs:element name="f" type="xs:int" fixed="3" minOccurs="0"/></xs:sequence><xs:attribute name="a" default="A"/><xs:attribute name="b" fixed="B"/></xs:complexType></xs:element></xs:schema>', [
 '<r><d/><f/></r>', '<r a="x"><d>1</d></r>',
], 1);
t('qname attr and id', '><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="i" type="xs:ID" maxOccurs="3"/></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 '<r><i>a</i><i>b</i></r>', '<r><i>a</i><i>a</i></r>',
]);
t('recursion', '><xs:element name="r" type="n"/><xs:complexType name="n"><xs:sequence><xs:element name="c" type="n" minOccurs="0" maxOccurs="unbounded"/></xs:sequence><xs:attribute name="v" type="xs:int"/></xs:complexType></xs:schema>', [
 '<r v="1"><c><c v="x"/></c><c/></r>',
]);
t('anyType', '><xs:element name="r"/><xs:element name="k" type="xs:int"/></xs:schema>', [
 '<r a="1">text<b><c/></b><k>x</k></r>',
]);
t('whitespace text', '><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a" minOccurs="0"/></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 "<r>\n  <a/>\n</r>", "<r>x<a/>y</r>", "<r><![CDATA[z]]></r>", "<r><![CDATA[ ]]></r>",
]);

chdir(sys_get_temp_dir());
t('prohibited/attr order', '><xs:element name="r"><xs:complexType><xs:attribute name="a" type="xs:int" use="prohibited"/><xs:attribute name="b" use="required"/></xs:complexType></xs:element></xs:schema>', [
 '<r a="1" b="1"/>', '<r/>', '<r a="x"/>',
]);
t('anyAttribute variants', '><xs:element name="r"><xs:complexType><xs:anyAttribute processContents="lax"/></xs:complexType></xs:element><xs:attribute name="g" type="xs:int"/></xs:schema>', [
 '<r x="1"/>', '<r xmlns:p="urn:p" p:g="1"/>',
]);
t('anyAttribute skip', '><xs:element name="r"><xs:complexType><xs:anyAttribute namespace="##any" processContents="skip"/></xs:complexType></xs:element></xs:schema>', [
 '<r x="1" y="2"/>',
]);
t('dates', '><xs:simpleType name="d"><xs:restriction base="xs:date"><xs:minInclusive value="2020-01-01"/><xs:maxExclusive value="2021-01-01"/></xs:restriction></xs:simpleType><xs:element name="r" type="d"/></xs:schema>', [
 '<r>2019-12-31</r>', '<r>2020-06-15</r>', '<r>2021-01-01</r>', '<r>2020-06-15Z</r>',
]);
t('decimals', '><xs:simpleType name="d"><xs:restriction base="xs:decimal"><xs:minExclusive value="0"/><xs:maxInclusive value="99.5"/></xs:restriction></xs:simpleType><xs:element name="r" type="d"/></xs:schema>', [
 '<r>0</r>', '<r>0.0001</r>', '<r>99.50</r>', '<r>99.51</r>', '<r>-1</r>', '<r>+5</r>', '<r>.5</r>', '<r>5.</r>',
]);
t('enum ints', '><xs:simpleType name="e"><xs:restriction base="xs:integer"><xs:enumeration value="1"/><xs:enumeration value="2"/></xs:restriction></xs:simpleType><xs:element name="r" type="e"/></xs:schema>', [
 '<r>01</r>', '<r>+2</r>', '<r>3</r>', '<r> 1 </r>',
]);
t('pattern forms', '><xs:simpleType name="p"><xs:restriction base="xs:string"><xs:pattern value="\d{3}-\d{4}"/><xs:pattern value="[a-z]+@[a-z]+\.com"/></xs:restriction></xs:simpleType><xs:element name="r" type="p"/></xs:schema>', [
 '<r>555-1234</r>', '<r>joe@x.com</r>', '<r>joe@x.org</r>', '<r>5551234</r>',
]);
t('whitespace', '><xs:simpleType name="t"><xs:restriction base="xs:token"><xs:minLength value="2"/></xs:restriction></xs:simpleType><xs:simpleType name="n"><xs:restriction base="xs:string"><xs:whiteSpace value="collapse"/><xs:enumeration value="a b"/></xs:restriction></xs:simpleType><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="t" type="t"/><xs:element name="n" type="n"/></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 "<r><t>  a  </t><n>a  b</n></r>", "<r><t>a</t><n>a\nb</n></r>",
]);
t('mixed content model', '><xs:element name="r"><xs:complexType mixed="true"><xs:choice maxOccurs="unbounded"><xs:element name="b"/><xs:element name="i"/></xs:choice></xs:complexType></xs:element></xs:schema>', [
 '<r>x<b/>y<i/>z</r>', '<r>x<u/></r>',
]);
t('multi-line tag', '><xs:element name="r"><xs:complexType><xs:attribute name="a" type="xs:int"/></xs:complexType></xs:element></xs:schema>', [
 "<r\n  a=\"x\"\n  b=\"y\"\n>\n</r>", "<r>\n\n<z>\n</z></r>",
]);
t('qname', '><xs:element name="r" type="xs:QName"/></xs:schema>', ['<r>a:b</r>', '<r>a:b:c</r>', '<r>1a</r>']);
t('bools', '><xs:element name="r" type="xs:boolean"/></xs:schema>', ['<r>true</r>', '<r>True</r>', '<r> 1 </r>']);
t('uri', '><xs:element name="r" type="xs:anyURI"/></xs:schema>', ['<r>http://a/b?c#d</r>', '<r>a b</r>', '<r>%zz</r>', '<r>a#b#c</r>', '<r>\\x</r>', '<r></r>']);
t('nonneg etc', '><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="u" type="xs:unsignedByte"/><xs:element name="n" type="xs:negativeInteger"/><xs:element name="l" type="xs:long"/></xs:sequence></xs:complexType></xs:element></xs:schema>', ['<r><u>256</u><n>0</n><l>9223372036854775808</l></r>', '<r><u>-1</u><n>-1</n><l>-9223372036854775808</l></r>']);
t('doc-level', '><xs:element name="r"/></xs:schema>', ['<?xml version="1.0"?><!-- c --><r/>', '<r/><!-- after -->']);
// include/import via files
file_put_contents('inc.xsd', '<?xml version="1.0"?><xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:simpleType name="small"><xs:restriction base="xs:int"><xs:maxInclusive value="9"/></xs:restriction></xs:simpleType></xs:schema>');
file_put_contents('main.xsd', '<?xml version="1.0"?><xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:include schemaLocation="inc.xsd"/><xs:element name="r" type="small"/></xs:schema>');
file_put_contents('imp.xsd', '<?xml version="1.0"?><xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:imp" xmlns:i="urn:imp" elementFormDefault="qualified"><xs:element name="x" type="xs:int"/></xs:schema>');
file_put_contents('main2.xsd', '<?xml version="1.0"?><xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:i="urn:imp"><xs:import namespace="urn:imp" schemaLocation="imp.xsd"/><xs:element name="r"><xs:complexType><xs:sequence><xs:element ref="i:x"/></xs:sequence></xs:complexType></xs:element></xs:schema>');
libxml_use_internal_errors(true);
foreach ([['main.xsd', '<r>5</r>'], ['main.xsd', '<r>50</r>'], ['main2.xsd', '<r><x xmlns="urn:imp">5</x></r>'], ['main2.xsd', '<r><x xmlns="urn:imp">q</x></r>']] as [$f, $x]) {
    $d = new DOMDocument; $d->loadXML($x);
    var_dump($d->schemaValidate($f));
    foreach (libxml_get_errors() as $e) echo "  C{$e->code} ", rtrim($e->message), "\n";
    libxml_clear_errors();
}
foreach (['main.xsd','inc.xsd','imp.xsd','main2.xsd'] as $f) unlink($f);
// warnings mode
libxml_use_internal_errors(false);
$d = new DOMDocument; $d->loadXML('<r><b/></r>');
var_dump($d->schemaValidateSource('<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a"/></xs:sequence></xs:complexType></xs:element></xs:schema>'));
var_dump($d->schemaValidateSource('<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:element name="r" type="nope"/></xs:schema>'));
var_dump($d->schemaValidateSource('<root/>'));
$e = new DOMDocument;
var_dump($e->schemaValidateSource('<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:element name="r"/></xs:schema>'));
libxml_use_internal_errors(true);
t('id attrs', '><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="i" maxOccurs="unbounded"><xs:complexType><xs:attribute name="id" type="xs:ID"/><xs:attribute name="ref" type="xs:IDREF"/></xs:complexType></xs:element></xs:sequence></xs:complexType></xs:element></xs:schema>', [
 '<r><i id="a"/><i id="b" ref="a"/></r>', '<r><i id="a"/><i id="a"/></r>', '<r><i ref="zz"/></r>', '<r><i id="1x"/></r>',
]);
