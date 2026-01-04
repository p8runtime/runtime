<?php
$file = __DIR__ . '/sha1.txt';
file_put_contents($file, 'data');
echo sha1_file($file);
