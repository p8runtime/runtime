<?php
// A page component: $props is what renderToResponse was given.
$name = htmlspecialchars($props['name'] ?? 'world');
?>
<!doctype html>
<html lang="en">
<body>
  <h1>Hello <?= $name ?></h1>
  <p>Rendered by PHP <?= PHP_VERSION ?> on p8, inside a Hono route.</p>
</body>
</html>

<style>
  h1 { color: #7c3aed; }
</style>
