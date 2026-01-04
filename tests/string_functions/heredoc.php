<?php
$heredoc = <<<TEXT
Heredoc line 1
Heredoc line 2
TEXT;

$nowdoc = <<<'TEXT'
Nowdoc keeps its literal content.
TEXT;

echo "Heredoc:\n" . rtrim($heredoc, "\n") . "\n";
echo "Nowdoc:\n" . rtrim($nowdoc, "\n") . "\n";
