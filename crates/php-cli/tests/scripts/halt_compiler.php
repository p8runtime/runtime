<?php
echo "before\n";
var_dump(__COMPILER_HALT_OFFSET__);
$d = file_get_contents(__FILE__, false, null, __COMPILER_HALT_OFFSET__);
var_dump($d);
__halt_compiler();raw } data ( $
more