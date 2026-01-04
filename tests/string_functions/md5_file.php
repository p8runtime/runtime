<?php
$file = __DIR__ . '/md5.txt';
file_put_contents($file, 'data');
echo md5_file($file);
