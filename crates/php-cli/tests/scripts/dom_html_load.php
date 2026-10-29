<?php
// DOMDocument::loadHTML: libxml2's HTML parser, its error recovery and its messages.
function dump(DOMNode $n, $d = 0) {
    $s = str_repeat(' ', $d) . $n->nodeType . ':' . $n->nodeName;
    if (in_array($n->nodeType, [3, 4, 7, 8])) $s .= '=' . json_encode($n->nodeValue);
    if ($n->nodeType == 1) foreach ($n->attributes as $a) $s .= " [{$a->name}=" . json_encode($a->value) . "]";
    echo $s, "\n";
    foreach ($n->childNodes as $c) dump($c, $d + 1);
}
function t($h, $o = 0) {
    $d = new DOMDocument();
    libxml_use_internal_errors(true);
    $r = $d->loadHTML($h, $o);
    echo "== ", json_encode($h), " options=$o\n";
    foreach (libxml_get_errors() as $e) echo "  level {$e->level} code {$e->code} line {$e->line} column {$e->column}: ", json_encode($e->message), "\n";
    libxml_clear_errors();
    dump($d);
    echo $d->saveHTML();
}

// implied html/head/body/p elements
t('<p>x</p>');
t('<title>x</title><p>y');
t('text <b>bold');
t('<head><title>t</title></head><body>b');
t('<html><head></head><body></body></html>');
t('<body><div>x</div></body><p>after body</p>');
t('<html><body>a</body></html><p>after html</p>');
t("   <p>leading space</p>   ");
// auto-closing and end tag priorities
t('<li>a<li>b');
t('<table><tr><td>a<td>b</table>');
t('<select><option>1<option>2</select>');
t('<p>a<div>b</div>c</p>');
t('<dl><dt>a<dd>b<dt>c</dl>');
t('<p>a<p>b<p>c');
t('<h1>a<h2>b</h1>c');
t('<b><i>x</b></i>');
t('</p>stray');
// void elements, attributes
t('<br/><hr><img src="a.png" alt=\'\'>');
t('<input disabled value=x><input checked="checked" selected>');
t('<div =x>y</div><div "a">z</div><a b=c d>e</a>');
t('<div a=1 a=2>x</div>');
t('<DIV CLASS=A ID=b>x</DIV>');
t('<p/>x<div/>text');
// unknown (HTML5) elements and prefixes
t('<custom-element data-x="1">y</custom-element><section><article>a</article></section>');
t('<svg:rect width=1><x:y/></svg:rect>');
t('<video controls><source src=a></video>');
// entities and character references
t('<p title="&amp; &lt; &quot; &nbsp; &copy; &#169; &#x41; &unknown; & &;">x</p>');
t('<p>&amp; &lt; &gt; &quot; &nbsp; &copy; &#169; &#x41; &unknown; & &; &#0; &#xD800; &#99999999; &eacute &#65</p>');
t('a < b > c & d');
// comments, processing instructions, doctype, raw text
t('<!-- a -- b --><p>x<!-- unfinished');
t('<!---><p>y</p><!--->');
t('<?php echo 1; ?><p>z</p>');
t('<p>x<![CDATA[y]]>z</p>');
t('<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd"><html><body>x</body></html>');
t('<!doctype html><title>t</title>');
t('<script>var a = "</div>"; if (a<b) { x = "<p>"; }</script><p>after');
t('<style>p > a { }</style><textarea> a <b> </textarea><pre>
 x</pre>');
// whitespace
t("<html>\n<head>\n<title>x</title>\n</head>\n<body>\n<p>a</p>\n\n<p>b</p>\n</body>\n</html>\n");
t('<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01//EN"><html>
<body>
 <p>a</p>
</body></html>');
// the options
foreach ([LIBXML_HTML_NOIMPLIED, LIBXML_HTML_NODEFDTD, LIBXML_HTML_NOIMPLIED | LIBXML_HTML_NODEFDTD, LIBXML_NOBLANKS, LIBXML_NOBLANKS | LIBXML_HTML_NOIMPLIED, LIBXML_RECOVER] as $o) {
    foreach (['<p>a</p><p>b</p>', 'text only', '<div>x</div> tail', "<ul>\n <li>a</li>\n <li>b</li>\n</ul>\n", '<!--c--><span>s</span>', '<title>t</title><p>x', '<tr><td>x'] as $c) t($c, $o);
}
// a document that is only blank, an empty string
$d = new DOMDocument();
var_dump($d->loadHTML(' '));
echo $d->saveHTML();
try { $d->loadHTML(''); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump($d->nodeType, $d->xmlStandalone, $d->xmlVersion, $d->encoding, $d->doctype->publicId);
// getElementById sees id attributes in an HTML document
$d->loadHTML('<div id="a"><a name="n">x</a><p id="b">y</p></div>');
var_dump($d->getElementById('b')->nodeName, $d->getElementById('n')->nodeName, $d->getElementById('zz'));
var_dump($d->getElementsByTagName('P')->length, $d->getElementsByTagName('p')->length);
// the errors become warnings without libxml_use_internal_errors; LIBXML_NOERROR hides them
libxml_use_internal_errors(false);
$d = new DOMDocument();
$d->loadHTML("<p>a</p>\n<foo>\n<bar>");
$d->loadHTML("<p>a</b>", LIBXML_NOERROR);
$e = libxml_get_last_error();
echo $e->level, ' ', $e->code, ' ', $e->line, ' ', $e->column, ' ', trim($e->message), "\n";
libxml_use_internal_errors(true);
libxml_clear_errors();
$d->loadHTML("<p>a</b>", LIBXML_NOERROR);
echo count(libxml_get_errors()), "\n";
// nesting deeper than 256 stops the parser unless LIBXML_PARSEHUGE
libxml_clear_errors();
foreach ([255, 256] as $n) {
    foreach ([0, LIBXML_PARSEHUGE] as $o) {
        $d = new DOMDocument();
        $d->loadHTML(str_repeat('<div>', $n) . 'x' . str_repeat('</div>', $n) . '<p>after</p>', $o);
        echo "n=$n o=$o ", $d->getElementsByTagName('div')->length, ' ', strlen($d->saveHTML()), "\n";
        foreach (libxml_get_errors() as $e) echo "  level {$e->level} code {$e->code}: ", json_encode($e->message), "\n";
        libxml_clear_errors();
    }
}
