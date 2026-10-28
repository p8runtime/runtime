<?php
// Literal text after an interpolation keeps its quotes; ${name} is $name.
$item = ['url' => 'U', 'format' => 'F'];
$s = 'S';
echo "url('{$item['url']}') format('{$item['format']}')", "\n";
echo "a{$item['url']}'b", "|a{$item["url"]}'b'", "|x{$s}'y'", "|x{$s}\"y\"", "|'$s'", "|x$item[url]'y'", "\n";
echo "esc: {$s}\t'\x41\101\u{263A}\\'", "\n";
echo <<<EOT
h('{$item['url']}')' "$s" \$x
EOT;
echo "\n", `echo '{$s}'`;
