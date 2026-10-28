<?php
function late_set(array &$a, $path, $v) { $ref = &$a; foreach ($path as $k) { $ref = &$ref[$k]; } $ref = $v; }
function late_value($a) { $a['changed'] = true; return count($a); }
