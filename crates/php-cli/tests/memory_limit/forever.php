<?php
function forever($n) { return forever($n + 1) + 1; }
register_shutdown_function(function () { echo "\nshutdown: ", error_get_last()['message'] ?? 'none', "\n"; });
try { echo forever(0), "\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
