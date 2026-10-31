<?php
// $props is the data the route passed to renderToResponse.
$name = htmlspecialchars($props['name'] ?? 'world');
?>
<!doctype html>
<html lang="en">
<body>
  <h1>Hello <?= $name ?></h1>
  <p>PHP rendered by p8 inside a Hono route.</p>
</body>
</html>
