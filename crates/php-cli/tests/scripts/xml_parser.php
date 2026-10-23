<?php
function run($xml, $default = false, $cdata = true) {
    $p = xml_parser_create();
    if ($cdata) xml_set_character_data_handler($p, fn($p, $d) => print("cdata[" . json_encode($d) . "]"));
    if ($default) xml_set_default_handler($p, fn($p, $d) => print("default[" . json_encode($d) . "]"));
    xml_set_element_handler($p, fn($p, $n, $a) => print("<$n " . json_encode($a) . ">"), fn($p, $n) => print("</$n>"));
    $r = xml_parse($p, $xml, true);
    echo " => ", var_export($r, true), " ", xml_get_error_code($p), " ", xml_error_string(xml_get_error_code($p)), " line ", xml_get_current_line_number($p), " byte ", xml_get_current_byte_index($p), "\n";
}
run('<a>x &lt; y &amp; z</a>');
run('<a>x &lt; y</a>', true);
run('<a>x &lt; y</a>', true, false);
run('<!DOCTYPE a [<!ENTITY e "hello <b>bold</b>">]><a>1 &e; 2 &e;</a>');
run('<!DOCTYPE a [<!ENTITY e "hello">]><a>1 &e; 2</a>', true);
run('<a>&undef; x</a>');
run("<a>line1\nline2\n  <b at='1 &lt; 2'/>\n</a>");
run('<a><b></a>');
run('<a x="1" x="2"/>');
run("<?xml version='1.0' encoding='ISO-8859-1'?><a>\xe9</a>");
run("<a>\xe9</a>");
run('');
run('<a>text');
run('<!-- c --><?pi data?><a><![CDATA[<cd>]]></a>', true);

// Chunks: events come as each construct completes.
$p = xml_parser_create_ns('UTF-8', '#');
xml_parser_set_option($p, XML_OPTION_CASE_FOLDING, 0);
xml_set_element_handler($p, fn($p, $n, $a) => print("open $n " . json_encode($a) . "\n"), fn($p, $n) => print("close $n\n"));
xml_set_character_data_handler($p, fn($p, $d) => print("text " . json_encode($d) . "\n"));
foreach (['<r xmlns="urn:a" xmlns:b="urn:b"><b:x b:at', 't="1">hel', 'lo</b:x><', 'y/></r>'] as $i => $chunk) {
    echo "chunk $i\n";
    var_dump(xml_parse($p, $chunk, $i == 3));
}
// xml_parse_into_struct.
$p = xml_parser_create();
xml_parse_into_struct($p, "<para><note>simple note</note><b>x</b>tail</para>", $vals, $index);
print_r($vals);
print_r($index);
var_dump(xml_parser_get_option($p, XML_OPTION_TARGET_ENCODING), xml_error_string(76));
try { new XMLParser(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { serialize(xml_parser_create()); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
