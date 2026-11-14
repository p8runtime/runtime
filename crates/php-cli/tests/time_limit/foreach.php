<?php
// Runs until max_execution_time stops it: a foreach over an array whose
// body has no loop of its own, so its back edge (its step) is where the
// limit is noticed.
foreach (range(1, 1000000) as $v) { $h = hash('sha256', str_repeat('x', 20000)); }
