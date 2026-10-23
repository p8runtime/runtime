<?php
$e = new ErrorException("ee", 1, E_WARNING, "file.php", 12);
echo $e->getSeverity(), " ", $e->getFile(), " ", $e->getLine(), "\n";
echo strlen([]);
