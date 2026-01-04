<?php
$fp = fopen('php://memory', 'r+');
vfprintf($fp, "%s", ['gild']);
rewind($fp);
echo stream_get_contents($fp);
