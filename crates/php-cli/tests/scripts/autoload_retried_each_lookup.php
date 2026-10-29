<?php
// A class still missing after the autoloaders is looked for again by the next lookup (PHP keeps no record of failures).
spl_autoload_register(function ($c) { echo "autoload $c\n"; });
var_dump(class_exists('Missing'), class_exists('Missing'), is_a('Missing', 'X', true), is_subclass_of('Missing', 'X'));
