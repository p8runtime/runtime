<?php
// DOMDocument::saveHTML and saveHTMLFile: HTMLtree.c's rules.
function build() {
    $d = new DOMDocument('1.0', 'UTF-8');
    $d->formatOutput = true;
    $h = $d->appendChild($d->createElement('html'));
    $b = $h->appendChild($d->createElement('body'));
    $dv = $b->appendChild($d->createElement('div'));
    $dv->appendChild($d->createTextNode('a & b <c> é'));
    $dv->setAttribute('data-x', 'q"r\'s & <t>');
    $b->appendChild($d->createElement('p', 'para'));
    $b->appendChild($d->createElement('p'));
    $b->appendChild($d->createElement('br'));
    $sc = $b->appendChild($d->createElement('script'));
    $sc->appendChild($d->createTextNode('if (a<b && c) {}'));
    $a = $b->appendChild($d->createElement('a', 'link'));
    $a->setAttribute('href', ' http://x.y/a b?c=d&e=é#f ');
    $a->setAttribute('name', 'a b');
    $b->appendChild($d->createComment(' c '));
    $b->appendChild($d->createCDATASection('cdata <x>'));
    $b->appendChild($d->createProcessingInstruction('php', 'echo 1;'));
    $b->appendChild($d->createEntityReference('nbsp'));
    $b->appendChild($d->createTextNode("t\r\nx\ty\x01z"));
    $b->appendChild($d->createElementNS('urn:x', 'x:el'))->setAttributeNS('urn:y', 'y:at', 'v');
    $b->appendChild($d->createElement('INPUT'))->setAttribute('CHECKED', 'checked');
    $b->appendChild($d->createElement('option'))->setAttribute('selected', '');
    $b->appendChild($d->createElement('textarea'));
    $b->appendChild($d->createElement('span'));
    return $d;
}
$d = build();
echo $d->saveHTML(), "|\n";
$d->formatOutput = false;
echo $d->saveHTML(), "|\n";
echo $d->saveHTML($d->getElementsByTagName('body')->item(0)), "|\n";
echo $d->saveHTML($d->getElementsByTagName('a')->item(0)->attributes->item(0)), "|\n";
echo $d->saveHTML($d->doctype ?? null), "|\n";
$f = $d->createDocumentFragment();
$f->appendXML('<i>x</i><u>y</u>');
echo $d->saveHTML($f), "|\n";
$other = new DOMDocument();
$other->loadHTML('<p>x</p>');
try { $d->saveHTML($other->documentElement); } catch (DOMException $e) { echo $e->getMessage(), "\n"; }

// a parsed document: the DOCTYPE line, format output, boolean attributes, URI escaping
$d = new DOMDocument();
$d->loadHTML('<!DOCTYPE html><html><head><title>T</title><meta charset="utf-8"></head><body><form action="a b.php?x=1&y=2" method=post><input type=checkbox checked disabled name="a b"><select><option selected value=1>One<option value=2>Two</select><textarea>t</textarea></form><a href="mailto:me@x.y?subject=a b">m</a><a href="javascript:void(0)">j</a><img src="  x y.png" alt="é"><p>one<br>two</p><pre>p
 re</pre><ul><li>a<li>b</ul><table><tr><td>c</table></body></html>');
echo $d->saveHTML();
$d->formatOutput = true;
echo $d->saveHTML();
echo $d->saveHTML($d->getElementsByTagName('ul')->item(0)), "|\n";
echo $d->saveHTML($d->getElementsByTagName('p')->item(0)), "|\n";
echo $d->saveXML();

// an XML document through saveHTML
$x = new DOMDocument();
$x->loadXML('<?xml version="1.0" encoding="UTF-8"?><r xmlns:p="urn:p"><p:a b="1">é</p:a><br/><e/><p>t</p></r>');
echo $x->saveHTML(), "|", $x->saveHTML($x->documentElement), "\n";

// the output encoding follows a Content-Type meta element; others are entities
foreach (['', '<meta http-equiv="Content-Type" content="text/html; charset=utf-8">', '<meta http-equiv="Content-Type" content="text/html; charset=ISO-8859-1">', '<meta charset="iso-8859-1">'] as $meta) {
    $d = new DOMDocument();
    $d->loadHTML("<html><head>$meta</head><body><p>caf\xc3\xa9 \xe2\x82\xac &amp; &lt; &eacute; &#8364; \xc3\xa9</p></body></html>");
    echo bin2hex($d->saveHTML()), "\n";
    echo bin2hex($d->saveHTML($d->getElementsByTagName('p')->item(0))), "\n";
}

// saveHTMLFile writes the file and records its encoding in the document
$dir = sys_get_temp_dir() . '/p8_dom_html_' . getmypid();
mkdir($dir);
$d = new DOMDocument();
$d->loadHTML('<html><head><title>x</title></head><body>é€</body></html>');
var_dump($d->saveHTMLFile("$dir/out.html"));
echo file_get_contents("$dir/out.html");
echo $d->saveHTML();
$d = new DOMDocument();
$d->loadHTML('<html><head><meta http-equiv="Content-Type" content="text/html; charset=ISO-8859-1"></head><body>é€</body></html>');
var_dump($d->saveHTMLFile("$dir/latin.html"));
echo bin2hex(file_get_contents("$dir/latin.html")), "\n";
var_dump(@$d->saveHTMLFile("$dir/missing/out.html"));
try { $d->saveHTMLFile(''); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
unlink("$dir/out.html");
unlink("$dir/latin.html");
rmdir($dir);
