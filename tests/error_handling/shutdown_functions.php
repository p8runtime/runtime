<?php
register_shutdown_function(function ($a) { echo "shutdown 1: $a\n"; register_shutdown_function(function () { print("nested\n"); }); }, "arg");
register_shutdown_function('printf', "shutdown %d\n", 2);
ob_start();
echo "body\n";
throw new Exception("late");
