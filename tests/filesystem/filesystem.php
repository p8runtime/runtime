<?php
// https://www.php.net/manual/en/function.file-put-contents.php
 $file = rtrim(sys_get_temp_dir(), DIRECTORY_SEPARATOR) . DIRECTORY_SEPARATOR . "gild_demo.txt";
file_put_contents($file, "Gild filesystem example\n");
echo "Written to $file\n";
echo "Contents: " . file_get_contents($file);
