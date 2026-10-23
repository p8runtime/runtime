<?php
function a($x) { b($x, "a long string argument"); }
function b($y, $s) { throw new RuntimeException("deep error"); }
a([1,2]);
