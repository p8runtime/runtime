<?php
// Text and markup: description, MIME type, MIME and encoding for strings.
$flags = [FILEINFO_NONE, FILEINFO_MIME_TYPE, FILEINFO_MIME, FILEINFO_MIME_ENCODING];

function show(string $label, string $data): void {
    global $flags;
    $row = [];
    foreach ($flags as $f) {
        $row[] = (new finfo($f))->buffer($data);
    }
    echo str_pad($label, 14), ' ', implode(' | ', $row), "\n";
}

$samples = [
    'plain'        => "Hello, world\n",
    'no newline'   => "Hello",
    'crlf'         => "line one\r\nline two\r\n",
    'cr'           => "line one\rline two\r",
    'mixed'        => "a\r\nb\nc\r",
    'long line'    => str_repeat('x', 400) . "\n",
    'utf8'         => "caf\xc3\xa9 cr\xc3\xa8me\n",
    'utf8 bom'     => "\xef\xbb\xbfhello\n",
    'latin1'       => "caf\xe9\n",
    'cp1252'       => "smart \x93quotes\x94\n",
    'utf16le'      => "\xff\xfeh\x00i\x00\n\x00",
    'utf16be'      => "\xfe\xff\x00h\x00i\x00\n",
    'escape'       => "a\x1b[31mred\x1b[0m\n",
    'overstrike'   => "a\x08a\n",
    'binary nul'   => "ab\x00\x01\x02cd",
    'html'         => "<html><head><title>x</title></head><body>hi</body></html>\n",
    'doctype'      => "<!DOCTYPE html>\n<html lang=\"en\"><body></body></html>\n",
    'html upper'   => "<HTML><BODY>x</BODY></HTML>",
    'html frag'    => "<p>just a paragraph</p>\n<table><tr><td>x</td></tr></table>\n",
    'html comment' => "<!-- a comment -->\n<html><body>x</body></html>\n",
    'sgml export'  => "<!-- wp:paragraph -->\n<p>x</p>\n<!-- /wp:paragraph -->\n",
    'xml'          => "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<root><a>1</a></root>\n",
    'xml single'   => "<?xml version='1.0'?><root/>",
    'xml no ver'   => "<?xml encoding=\"UTF-8\"?><root/>",
    'xhtml'        => "<?xml version=\"1.0\"?>\n<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"x.dtd\">\n<html xmlns=\"http://www.w3.org/1999/xhtml\"></html>\n",
    'rss'          => "<?xml version=\"1.0\"?>\n<rss version=\"2.0\"><channel></channel></rss>\n",
    'sitemap'      => "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"></urlset>\n",
    'svg bare'     => "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"></svg>",
    'svg xml'      => "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>\n",
    'svg doctype'  => "<?xml version=\"1.0\"?>\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"x.dtd\">\n<svg></svg>",
    'svg comment'  => "<!-- generator -->\n<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
    'php'          => "<?php\necho 1;\n",
    'php short'    => "<?\necho 1;\n",
    'php html'     => "<?php echo 1; ?>\n<html><body></body></html>\n",
    'html php'     => "<html><body><?php echo 1; ?></body></html>\n",
    'json obj'     => "{\"a\": 1, \"b\": [true, false, null]}",
    'json arr'     => "[1, 2, 3]\n",
    'json utf8'    => "{\"name\": \"caf\xc3\xa9\"}\n",
    'json scalar'  => "42",
    'json bad'     => "{\"a\": }",
    'ndjson'       => "{\"a\":1}\n{\"b\":2}\n",
    'csv'          => "id,name,qty\n1,apple,3\n2,pear,4\n3,fig,5\n",
    'csv quoted'   => "a,b\n\"x,y\",2\n\"z\",3\n",
    'csv utf8'     => "id,name\n1,caf\xc3\xa9\n2,thé\n3,x\n",
    'sh'           => "#!/bin/sh\necho hi\n",
    'bash'         => "#!/bin/bash\necho hi\n",
    'env bash'     => "#!/usr/bin/env bash\necho hi\n",
    'python env'   => "#!/usr/bin/env python3\nprint(1)\n",
    'perl'         => "#!/usr/bin/perl\nprint 1;\n",
    'node'         => "#!/usr/bin/env node\nconsole.log(1);\n",
    'js strict'    => "'use strict';\nconsole.log(1);\n",
    'js func'      => "var a = function(x) { return x; };\n",
    'js module'    => "module.exports = { a: 1 };\n",
    'css'          => "body { color: red; }\n",
    'c'            => "#include <stdio.h>\nint main(void) { return 0; }\n",
    'rtf'          => "{\\rtf1\\ansi\\ansicpg1252\\deff0 {\\fonttbl{\\f0 Arial;}} Hello}",
    'webvtt'       => "WEBVTT\n\n00:00.000 --> 00:01.000\nhi\n",
    'po'           => "msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\nmsgid \"a\"\nmsgstr \"b\"\n",
    'po lead nl'   => "\nmsgid \"a\"\nmsgstr \"b\"\n",
    'email'        => "From: a@example.com\nTo: b@example.com\nSubject: hi\n\nbody\n",
    'diff'         => "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n",
    'ini'          => "[section]\nkey=value\n",
    'markdown'     => "# Title\n\nSome *text*.\n",
    'x'            => "x",
    'empty'        => "",
    'two bytes'    => "ab",
];
foreach ($samples as $label => $data) {
    show($label, $data);
}

// FILEINFO_CONTINUE lists every answer
echo "\ncontinue:\n";
foreach (['php html' => $samples['php html'], 'html php' => $samples['html php'], 'csv' => $samples['csv'], 'sh' => $samples['sh']] as $label => $data) {
    echo str_pad($label, 14), ' ', json_encode((new finfo(FILEINFO_CONTINUE))->buffer($data)), ' | ', json_encode((new finfo(FILEINFO_MIME_TYPE | FILEINFO_CONTINUE))->buffer($data)), "\n";
}
