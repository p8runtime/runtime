<?php
echo substr('hello', -2) . "\n";
echo substr('hello', 1, -1) . "\n";

echo (strpos('hello', 'z') === false ? 'false' : 'true') . "\n";

print_r(explode('-', 'a-b-c'));

echo str_repeat('x', 0) . "\n";

echo str_pad('hi', 5, '.', 2) . "\n";

echo trim('  hello..', " .") . "\n";
