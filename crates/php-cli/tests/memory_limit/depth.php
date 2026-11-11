<?php
function depth($n) { return $n ? depth($n - 1) + 1 : 0; }
echo depth(1000000), "\n";
