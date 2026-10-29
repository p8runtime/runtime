<?php
$dir = sys_get_temp_dir() . '/p8_dom_xinclude_' . getmypid();
@mkdir($dir);
@mkdir("$dir/sub");
file_put_contents("$dir/part.xml", '<part a="1"><b>x</b></part>');
file_put_contents("$dir/note.txt", "plain & text\n");
file_put_contents("$dir/latin.txt", "caf\xe9\n");
file_put_contents("$dir/doc.xml", '<?xml version="1.0"?><!-- c --><r2 xmlns:z="urn:z"><k id="1"/><z:k id="2"/></r2>');
file_put_contents("$dir/sub/inner.xml", '<inner xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="deep.xml"/><xi:include href="../part.xml"/></inner>');
file_put_contents("$dir/sub/deep.xml", '<deep/>');
file_put_contents("$dir/loop.xml", '<l xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="loop.xml"/></l>');
file_put_contents("$dir/bad.xml", '<a>');
file_put_contents("$dir/fb.xml", '<f xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="gone.xml"><xi:fallback>FB</xi:fallback></xi:include><xi:include href="part.xml"/></f>');

set_error_handler(function ($no, $msg) use ($dir) {
    echo 'Warning: ', str_replace($dir, 'DIR', $msg), "\n";
    return true;
});

$X = 'xmlns:xi="http://www.w3.org/2001/XInclude"';

function run(string $title, string $xml, int $options = 0, bool $internal = true): void
{
    global $dir;
    echo "-- $title\n";
    libxml_use_internal_errors($internal);
    $d = new DOMDocument();
    $d->loadXML($xml);
    $d->documentURI = "$dir/main.xml";
    var_dump($d->xinclude($options));
    echo str_replace($dir, 'DIR', $d->saveXML()), "\n";
    foreach (libxml_get_errors() as $e) {
        echo $e->level, ' ', $e->code, ' ', trim(str_replace($dir, 'DIR', $e->message)), ' | ', str_replace($dir, 'DIR', $e->file), ':', $e->line, "\n";
    }
    libxml_clear_errors();
}

$d = new DOMDocument();
$d->loadXML('<root/>');
var_dump($d->xinclude());
var_dump((new DOMDocument())->xinclude());
var_dump($d->xinclude(PHP_INT_MAX));

run('relative href, xml and text', "<root $X><xi:include href='part.xml'/><xi:include href='sub/inner.xml'/><xi:include href='note.txt' parse='text'/></root>");
run('text encoding', "<root $X><xi:include href='latin.txt' parse='text' encoding='ISO-8859-1'/></root>");
run('unsupported encoding', "<root $X><xi:include href='latin.txt' parse='text' encoding='nope'/></root>");
run('2003 namespace', "<root xmlns:xi='http://www.w3.org/2003/XInclude'><xi:include href='sub/deep.xml'/></root>");
run('fallback', "<root $X><xi:include href='gone.xml'><xi:fallback><f/>tail</xi:fallback></xi:include></root>");
run('no fallback', "<root $X><xi:include href='gone.xml'/></root>");
run('fallback with an include, and one that fails', "<root $X><xi:include href='gone.xml'><xi:fallback><xi:include href='part.xml'/><xi:include href='gone2.xml'/></xi:fallback></xi:include></root>");
run('nested fallback in included file', "<root $X><xi:include href='fb.xml'/></root>");
run('local recursion', "<root $X><xi:include href='main.xml'/></root>");
run('recursion through a file', "<root $X><xi:include href='loop.xml'/></root>");
run('not well-formed', "<root $X><xi:include href='bad.xml'/></root>");
run('bad parse value', "<root $X><xi:include href='part.xml' parse='foo'/></root>");
run('fallback outside include', "<root $X><xi:fallback/></root>");
run('two fallbacks', "<root $X><xi:include href='gone.xml'><xi:fallback/><xi:fallback/></xi:include></root>");
run('include in include', "<root $X><xi:include href='part.xml'><xi:include href='sub/deep.xml'/></xi:include></root>");
run('one failure, one success', "<root $X><xi:include href='gone.xml'/><xi:include href='part.xml'/></root>");
run('NOBASEFIX', "<root $X><xi:include href='sub/inner.xml'/></root>", 1 << 18);
run('xml:base on the include', "<root $X><xi:include href='part.xml' xml:base='elsewhere/'/></root>");
run('xml:base on an ancestor', "<root $X xml:base='sub/'><x><xi:include href='../part.xml'/></x></root>");
run('whole document with a comment', "<root $X><xi:include href='doc.xml'/></root>");
run('xpointer(): elements', "<root $X><xi:include href='doc.xml' xpointer='xpointer(/r2/*)'/></root>");
run('xpointer in href', "<root $X><xi:include href='part.xml#xpointer(/part/b)'/></root>");
run('element() child sequence', "<root $X><xi:include href='doc.xml' xpointer='element(/1/2)'/></root>");
run('xpointer selects nothing', "<root $X><xi:include href='doc.xml' xpointer='xpointer(/zzz)'/></root>");
run('xpointer names no id', "<root $X><xi:include href='doc.xml' xpointer='nothere'/></root>");
run('xpointer to the document itself', "<root $X><a><b/></a><xi:include xpointer='xpointer(/root/a)'/></root>");
run('root is the include', "<xi:include $X href='part.xml'/>");
run('warnings', "<root $X><xi:include href='gone.xml'/><xi:include href='bad.xml'><xi:fallback/></xi:include></root>", 0, false);

foreach (['part.xml', 'note.txt', 'latin.txt', 'doc.xml', 'sub/inner.xml', 'sub/deep.xml', 'loop.xml', 'bad.xml', 'fb.xml'] as $f) {
    unlink("$dir/$f");
}
rmdir("$dir/sub");
rmdir($dir);
