<?php
// loadHTML and character encodings: ISO-8859-1 unless told otherwise, the idioms WordPress and others use.
function t($h, $o = 0) {
    $d = new DOMDocument();
    libxml_use_internal_errors(true);
    $d->loadHTML($h, $o);
    echo "== ", bin2hex(substr($h, 0, 48)), " options=$o\n";
    foreach (libxml_get_errors() as $e) echo "  level {$e->level} code {$e->code} line {$e->line} column {$e->column}: ", json_encode($e->message), "\n";
    libxml_clear_errors();
    echo "encoding=", var_export($d->encoding, true), "\n";
    echo $d->saveHTML();
    echo bin2hex($d->documentElement ? $d->saveHTML($d->documentElement) : ''), "\n";
}
t("<p>caf\xe9</p>");
t("<p>caf\xc3\xa9</p>");
t("<meta charset=\"utf-8\"><p>caf\xc3\xa9 \xe2\x82\xac</p>");
t("<meta charset=utf-8><p>caf\xe9</p>");
t("<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"><p>caf\xc3\xa9</p>");
t("<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8;\"><p>caf\xc3\xa9</p>");
t("<meta http-equiv=\"Content-Type\" content=\"text/html; charset=ISO-8859-1\"><p>caf\xe9 \xc3\xa9</p>");
t("<meta http-equiv=\"Content-Type\" content=\"text/html; charset=windows-1252\"><p>caf\xe9 \x80</p>");
t("<meta http-equiv=\"Content-Type\" content=\"text/html; charset=iso-8859-2\"><p>caf\xe9 \xb1</p>");
t("<meta charset=bogus><p>caf\xe9</p>");
t("<html><head><title>\xc3\xa9</title><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></head></html>");
t("\xef\xbb\xbf<p>bom \xc3\xa9</p>");
t("\xff\xfe<\0p\0>\0\xe9\0<\0/\0p\0>\0");
// the prefix that tells libxml2 the input is UTF-8
t("<?xml encoding=\"UTF-8\">\n<p>caf\xc3\xa9 \xe2\x82\xac \xf0\x9f\x98\x80</p>");
t("<?xml encoding=\"UTF-8\">\n<p>caf\xe9</p>");
t("<?xml version=\"1.0\" encoding=\"UTF-8\"?><p>caf\xc3\xa9</p>", LIBXML_HTML_NOIMPLIED | LIBXML_HTML_NODEFDTD);
t("<p class=\"caf\xc3\xa9\" title=\"\xe9\">x</p>");
t("<!-- caf\xc3\xa9 --><!-- \xe9 --><p>x</p>");
t("<p>a\0b</p>");
t("<p>\x01\x02 ctrl</p>");

// WordPress: fragments, UTF-8 and a single node back
$d = new DOMDocument();
libxml_use_internal_errors(true);
$d->loadHTML('<?xml encoding="UTF-8">' . '<div class="a"><p>é “q” &amp; <img src="x.jpg"><br><span>x</span></p></div><div>2</div>', LIBXML_HTML_NOIMPLIED | LIBXML_HTML_NODEFDTD);
echo $d->saveHTML(), "\n";
foreach ($d->childNodes as $c) echo get_class($c), ':', $c->nodeName, ' ', $d->saveHTML($c), "\n";
$d->encoding = 'UTF-8';
echo $d->saveHTML(), '|', $d->saveHTML($d->documentElement), "\n";

// HTML5 markup in a loaded document, the errors collected
$d = new DOMDocument();
libxml_use_internal_errors(true);
$d->loadHTML('<!DOCTYPE html><html><body><header><nav>x</nav></header><main><article><figure><img src=a><figcaption>c</figcaption></figure></article></main><footer>f</footer></body></html>');
foreach (libxml_get_errors() as $e) echo trim($e->message), ' @', $e->line, ':', $e->column, "\n";
libxml_clear_errors();
echo $d->saveHTML(), "\n";

// loadHTMLFile: errors name the file
$dir = sys_get_temp_dir() . '/p8_dom_html_' . getmypid();
mkdir($dir);
file_put_contents("$dir/f.html", "<html>\n<body>\n<foo>x</foo>\n</body></html>");
$d = new DOMDocument();
var_dump($d->loadHTMLFile("$dir/f.html"));
foreach (libxml_get_errors() as $e) echo $e->line, ':', basename($e->file), ':', trim($e->message), "\n";
libxml_clear_errors();
echo basename($d->documentURI), "\n";
echo $d->saveHTML();
libxml_use_internal_errors(false);
var_dump(@$d->loadHTMLFile("$dir/none.html"));
try { $d->loadHTMLFile(''); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $d->loadHTMLFile("a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
unlink("$dir/f.html");
rmdir($dir);
