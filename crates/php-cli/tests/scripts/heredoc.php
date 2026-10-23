<?php
// Heredoc and nowdoc bodies: closing indentation, the final newline,
// interpolation at line starts, blank lines, escapes.
$name = "World";
$n = 3;
$a = <<<EOT
Hello $name
  indented {$name}s
EOT;
var_dump($a);
$b = <<<EOT
    $name at a line start
      two more spaces

    after a blank line, n=$n
    EOT;
var_dump($b);
$c = <<<'EOT'
    raw $name \t \\
    EOT;
var_dump($c);
$d = <<<"EOT"
	tab-indented $name
	EOT;
var_dump($d);
$e = <<<EOT
\\
EOT;
var_dump($e);
$f = <<<EOT
EOT;
var_dump($f);
echo str_repeat(<<<X
  in a call
  X, 2), "\n";
var_dump(<<<EOT
  a\tb\x41
  EOT . "!");
