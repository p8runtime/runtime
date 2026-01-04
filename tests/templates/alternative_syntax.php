<?php
$show = isset($argv) && count($argv) > 1 ? $argv[1] === "true" : true;
?>
<!DOCTYPE html>
<html>
<head>
  <title>Alternative Syntax</title>
</head>
<body><?php if ($show): ?>  <p>This row renders when the expression is true.</p><?php else: ?>  <p>Otherwise this row renders.</p><?php endif; ?></body>
</html>
