<?php
// With no output at all, the callback runs when the script ends.
header_register_callback(function () { fwrite(STDOUT, "[cb at end]\n"); });
$x = 1;
