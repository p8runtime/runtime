<?php
function cb($v) { throw new RuntimeException("cb $v"); }
array_map('cb', [1]);
