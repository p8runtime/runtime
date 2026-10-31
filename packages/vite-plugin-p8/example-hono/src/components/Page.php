<?php
// A page: the request is PHP's own, as under php -S.
$name = htmlspecialchars($_GET['name'] ?? 'world');
header('X-Greeting: hello');
?>
<!doctype html>
<html lang="en">
<body>
  <h1>Hello <?= $name ?></h1>
  <p><?= htmlspecialchars($_SERVER['REQUEST_METHOD']) ?> <?= htmlspecialchars($_SERVER['REQUEST_URI']) ?>,
     rendered by P8 inside a Hono route.</p>
</body>
</html>
