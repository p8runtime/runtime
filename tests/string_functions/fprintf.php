<?php
$fp = fopen('php://memory', 'r+');
fprintf($fp, "%s %d", 'gild', 1);
rewind($fp);
echo stream_get_contents($fp);
