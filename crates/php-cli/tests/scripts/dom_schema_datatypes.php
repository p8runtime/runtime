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
}
$xsd = '<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
 <xs:simpleType name="color"><xs:restriction base="xs:string"><xs:enumeration value="red"/><xs:enumeration value="green"/></xs:restriction></xs:simpleType>
 <xs:simpleType name="small"><xs:restriction base="xs:integer"><xs:minInclusive value="1"/><xs:maxExclusive value="10"/></xs:restriction></xs:simpleType>
 <xs:simpleType name="code"><xs:restriction base="xs:string"><xs:pattern value="[A-Z]{3}[0-9]+"/><xs:maxLength value="6"/></xs:restriction></xs:simpleType>
 <xs:simpleType name="ints"><xs:list itemType="xs:integer"/></xs:simpleType>
 <xs:simpleType name="intorcolor"><xs:union memberTypes="xs:integer color"/></xs:simpleType>
 <xs:simpleType name="dec"><xs:restriction base="xs:decimal"><xs:totalDigits value="4"/><xs:fractionDigits value="2"/></xs:restriction></xs:simpleType>
 <xs:simpleType name="len"><xs:restriction base="xs:string"><xs:length value="3"/></xs:restriction></xs:simpleType>
 <xs:simpleType name="minlen"><xs:restriction base="xs:string"><xs:minLength value="2"/></xs:restriction></xs:simpleType>
 <xs:simpleType name="localtest"><xs:restriction base="xs:integer"/></xs:simpleType>
 <xs:element name="r">
  <xs:complexType>
   <xs:sequence>
    <xs:element name="color" type="color" minOccurs="0"/>
    <xs:element name="small" type="small" minOccurs="0"/>
    <xs:element name="code" type="code" minOccurs="0"/>
    <xs:element name="ints" type="ints" minOccurs="0"/>
    <xs:element name="u" type="intorcolor" minOccurs="0"/>
    <xs:element name="dec" type="dec" minOccurs="0"/>
    <xs:element name="len" type="len" minOccurs="0"/>
    <xs:element name="minlen" type="minlen" minOccurs="0"/>
    <xs:element name="anon" minOccurs="0"><xs:simpleType><xs:restriction base="xs:string"><xs:enumeration value="a"/></xs:restriction></xs:simpleType></xs:element>
    <xs:element name="anon2" minOccurs="0"><xs:simpleType><xs:restriction base="xs:int"><xs:maxInclusive value="5"/></xs:restriction></xs:simpleType></xs:element>
    <xs:element name="lt" type="localtest" minOccurs="0"/>
    <xs:element name="b" type="xs:byte" minOccurs="0"/>
    <xs:element name="dt" type="xs:dateTime" minOccurs="0"/>
    <xs:element name="d" type="xs:date" minOccurs="0"/>
    <xs:element name="fl" type="xs:float" minOccurs="0"/>
    <xs:element name="hb" type="xs:hexBinary" minOccurs="0"/>
    <xs:element name="tok" type="xs:NCName" minOccurs="0"/>
    <xs:element name="ps" type="xs:positiveInteger" minOccurs="0"/>
    <xs:element name="lang" type="xs:language" minOccurs="0"/>
    <xs:element name="dur" type="xs:duration" minOccurs="0"/>
    <xs:element name="nm" type="xs:NMTOKENS" minOccurs="0"/>
   </xs:sequence>
   <xs:attribute name="c" type="color"/>
   <xs:attribute name="n" type="small"/>
   <xs:attribute name="l" type="ints"/>
   <xs:attribute name="u" type="intorcolor"/>
   <xs:attribute name="f" type="xs:string" fixed="abc"/>
   <xs:attribute name="d" type="xs:string" default="dd"/>
  </xs:complexType>
 </xs:element>
</xs:schema>';
$cases = [
 '<r><color>blue</color></r>',
 '<r><small>0</small></r>', '<r><small>10</small></r>', '<r><small>abc</small></r>',
 '<r><code>abc1</code></r>', '<r><code>ABC1234567</code></r>',
 '<r><ints>1 2 x</ints></r>', '<r><ints>1 2 3</ints></r>',
 '<r><u>zzz</u></r>', '<r><dec>12345</dec></r>', '<r><dec>1.234</dec></r>', '<r><dec>1.2.3</dec></r>',
 '<r><len>ab</len></r>', '<r><minlen>a</minlen></r>',
 '<r><anon>b</anon></r>', '<r><anon2>7</anon2></r>', '<r><anon2>x</anon2></r>', '<r><lt>q</lt></r>',
 '<r><b>200</b></r>', '<r><dt>2020-13-01T00:00:00</dt></r>', '<r><d>2020-02-30</d></r>', '<r><fl>abc</fl></r>',
 '<r><hb>abc</hb></r>', '<r><tok>1a</tok></r>', '<r><ps>0</ps></r>', '<r><lang>toolonglang1</lang></r>', '<r><dur>P</dur></r>', '<r><nm></nm></r>',
 '<r c="blue" n="11" l="a b" u="zz" f="x"/>',
 '<r><color> red </color></r>', '<r><small> 5 </small></r>', '<r><code> ABC1</code></r>', '<r><color/></r>',
];
foreach ($cases as $c) { echo "== $c\n"; v($c, $xsd); }
