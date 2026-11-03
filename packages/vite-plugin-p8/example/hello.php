<?php
require __DIR__ . '/lib/greet.php';

$name = $_POST['name'] ?? $_GET['name'] ?? 'world';
if ($_SERVER['REQUEST_METHOD'] === 'POST') {
    setcookie('last_name', $name);
}
header('X-Rendered-By: p8');
?>
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>hello.php</title>
</head>
<body>
  <h1><?= htmlspecialchars(greet($name)) ?></h1>
  <form method="post" action="<?= htmlspecialchars($_SERVER['REQUEST_URI']) ?>">
    <input name="name" value="<?= htmlspecialchars($name) ?>">
    <button>Greet</button>
  </form>
  <dl>
<?php foreach (['REQUEST_METHOD', 'REQUEST_URI', 'QUERY_STRING', 'SCRIPT_NAME', 'SCRIPT_FILENAME', 'PATH_INFO', 'DOCUMENT_ROOT', 'SERVER_SOFTWARE', 'HTTP_USER_AGENT'] as $key): ?>
    <dt><?= $key ?></dt><dd id="<?= $key ?>"><?= htmlspecialchars((string) ($_SERVER[$key] ?? '')) ?></dd>
<?php endforeach ?>
    <dt>PHP_SAPI</dt><dd id="PHP_SAPI"><?= PHP_SAPI ?></dd>
    <dt>last_name cookie</dt><dd id="cookie"><?= htmlspecialchars($_COOKIE['last_name'] ?? '') ?></dd>
  </dl>
</body>
</html>
